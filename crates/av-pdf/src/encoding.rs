//! A simple font's codes as text: the standard encodings (WinAnsi, MacRoman,
//! Standard) and the glyph names a font's `/Differences` give, for Western
//! scripts (ASCII, Latin-1, Windows-1252's extras, ligatures) and the
//! `uniXXXX` and `uXXXX` names.

use av_utils::charsets::windows1252_char;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Base {
    Standard,
    WinAnsi,
    MacRoman,
}

// ----------------------------------------------------------------------------
/// The glyph names of 0x20 to 0x7E.
#[rustfmt::skip]
const ASCII: [&str; 95] = [
    "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand",
    "quotesingle", "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period",
    "slash", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    "colon", "semicolon", "less", "equal", "greater", "question", "at",
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
    "S", "T", "U", "V", "W", "X", "Y", "Z",
    "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "grave",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
    "s", "t", "u", "v", "w", "x", "y", "z",
    "braceleft", "bar", "braceright", "asciitilde",
];

// ----------------------------------------------------------------------------
/// The glyph names of 0xA0 to 0xFF.
#[rustfmt::skip]
const LATIN1: [&str; 96] = [
    "nbspace", "exclamdown", "cent", "sterling", "currency", "yen", "brokenbar", "section",
    "dieresis", "copyright", "ordfeminine", "guillemotleft", "logicalnot", "sfthyphen",
    "registered", "macron", "degree", "plusminus", "twosuperior", "threesuperior", "acute", "mu",
    "paragraph", "periodcentered", "cedilla", "onesuperior", "ordmasculine", "guillemotright",
    "onequarter", "onehalf", "threequarters", "questiondown",
    "Agrave", "Aacute", "Acircumflex", "Atilde", "Adieresis", "Aring", "AE", "Ccedilla",
    "Egrave", "Eacute", "Ecircumflex", "Edieresis", "Igrave", "Iacute", "Icircumflex",
    "Idieresis", "Eth", "Ntilde", "Ograve", "Oacute", "Ocircumflex", "Otilde", "Odieresis",
    "multiply", "Oslash", "Ugrave", "Uacute", "Ucircumflex", "Udieresis", "Yacute", "Thorn",
    "germandbls",
    "agrave", "aacute", "acircumflex", "atilde", "adieresis", "aring", "ae", "ccedilla",
    "egrave", "eacute", "ecircumflex", "edieresis", "igrave", "iacute", "icircumflex",
    "idieresis", "eth", "ntilde", "ograve", "oacute", "ocircumflex", "otilde", "odieresis",
    "divide", "oslash", "ugrave", "uacute", "ucircumflex", "udieresis", "yacute", "thorn",
    "ydieresis",
];

// ----------------------------------------------------------------------------
/// Other glyph names: Windows-1252's extras, ligatures, accents.
#[rustfmt::skip]
const OTHERS: [(&str, u16); 44] = [
    ("Euro", 0x20AC), ("quotesinglbase", 0x201A), ("florin", 0x0192), ("quotedblbase", 0x201E),
    ("ellipsis", 0x2026), ("dagger", 0x2020), ("daggerdbl", 0x2021), ("circumflex", 0x02C6),
    ("perthousand", 0x2030), ("Scaron", 0x0160), ("guilsinglleft", 0x2039), ("OE", 0x0152),
    ("Zcaron", 0x017D), ("quoteleft", 0x2018), ("quoteright", 0x2019),
    ("quotedblleft", 0x201C), ("quotedblright", 0x201D), ("bullet", 0x2022),
    ("endash", 0x2013), ("emdash", 0x2014), ("tilde", 0x02DC), ("trademark", 0x2122),
    ("scaron", 0x0161), ("guilsinglright", 0x203A), ("oe", 0x0153), ("zcaron", 0x017E),
    ("Ydieresis", 0x0178), ("ff", 0xFB00), ("fi", 0xFB01), ("fl", 0xFB02), ("ffi", 0xFB03),
    ("ffl", 0xFB04), ("dotlessi", 0x0131), ("Lslash", 0x0141), ("lslash", 0x0142),
    ("minus", 0x2212), ("fraction", 0x2044), ("breve", 0x02D8), ("dotaccent", 0x02D9),
    ("ring", 0x02DA), ("hungarumlaut", 0x02DD), ("ogonek", 0x02DB), ("caron", 0x02C7),
    ("nonbreakingspace", 0x00A0),
];

// ----------------------------------------------------------------------------
/// StandardEncoding's codes from 0xA1 where it differs from Latin-1.
#[rustfmt::skip]
const STANDARD_HIGH: [(u8, u16); 54] = [
    (0xA1, 0x00A1), (0xA2, 0x00A2), (0xA3, 0x00A3), (0xA4, 0x2044), (0xA5, 0x00A5),
    (0xA6, 0x0192), (0xA7, 0x00A7), (0xA8, 0x00A4), (0xA9, 0x0027), (0xAA, 0x201C),
    (0xAB, 0x00AB), (0xAC, 0x2039), (0xAD, 0x203A), (0xAE, 0xFB01), (0xAF, 0xFB02),
    (0xB1, 0x2013), (0xB2, 0x2020), (0xB3, 0x2021), (0xB4, 0x00B7), (0xB6, 0x00B6),
    (0xB7, 0x2022), (0xB8, 0x201A), (0xB9, 0x201E), (0xBA, 0x201D), (0xBB, 0x00BB),
    (0xBC, 0x2026), (0xBD, 0x2030), (0xBF, 0x00BF), (0xC1, 0x0060), (0xC2, 0x00B4),
    (0xC3, 0x02C6), (0xC4, 0x02DC), (0xC5, 0x00AF), (0xC6, 0x02D8), (0xC7, 0x02D9),
    (0xC8, 0x00A8), (0xCA, 0x02DA), (0xCB, 0x00B8), (0xCD, 0x02DD), (0xCE, 0x02DB),
    (0xCF, 0x02C7), (0xD0, 0x2014), (0xE1, 0x00C6), (0xE3, 0x00AA), (0xE8, 0x0141),
    (0xE9, 0x00D8), (0xEA, 0x0152), (0xEB, 0x00BA), (0xF1, 0x00E6), (0xF5, 0x0131),
    (0xF8, 0x0142), (0xF9, 0x00F8), (0xFA, 0x0153), (0xFB, 0x00DF),
];

// ----------------------------------------------------------------------------
/// MacRomanEncoding's 0x80 to 0xFF.
#[rustfmt::skip]
const MAC_ROMAN: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4,
    0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF,
    0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020,
    0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4,
    0x00A8, 0x2260, 0x00C6, 0x00D8, 0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202,
    0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1,
    0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3,
    0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0x00B7, 0x201A,
    0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC,
    0x00D3, 0x00D4, 0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF,
    0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

// ----------------------------------------------------------------------------
/// The character `code` stands for in `base`.
pub fn base_char(base: Base, code: u8) -> Option<char> {
    match (base, code) {
        (Base::Standard, 0x27) => Some('\u{2019}'),
        (Base::Standard, 0x60) => Some('\u{2018}'),
        (_, 0x20..=0x7e) => Some(code as char),
        (Base::WinAnsi, 0x80..=0xff) => Some(windows1252_char(code)),
        (Base::MacRoman, 0x80..=0xff) => char::from_u32(MAC_ROMAN[code as usize - 0x80] as u32),
        (Base::Standard, _) => STANDARD_HIGH
            .iter()
            .find(|(c, _)| *c == code)
            .and_then(|(_, u)| char::from_u32(*u as u32)),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
/// The text of the glyph `name`: a known name, `uniXXXX` (one or more
/// UTF-16 units), `uXXXX`; a suffix (`a.sc`) dropped, a ligature's parts
/// (`f_i`) joined.
pub fn glyph_text(name: &str) -> Option<String> {
    let name = name.split('.').next().unwrap_or(name);
    if name.contains('_') {
        return name.split('_').map(glyph_text).collect();
    }
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 {
            let units: Option<Vec<u16>> = hex
                .as_bytes()
                .chunks(4)
                .map(|unit| u16::from_str_radix(std::str::from_utf8(unit).ok()?, 16).ok())
                .collect();
            return Some(char::decode_utf16(units?).filter_map(Result::ok).collect());
        }
    }
    if let Some(hex) = name.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) {
            if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                return Some(c.to_string());
            }
        }
    }
    let code = if let Some(i) = ASCII.iter().position(|n| *n == name) {
        0x20 + i as u32
    } else if let Some(i) = LATIN1.iter().position(|n| *n == name) {
        0xA0 + i as u32
    } else {
        OTHERS.iter().find(|(n, _)| *n == name)?.1 as u32
    };
    char::from_u32(code).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn base_encodings_differ_where_they_should() {
        assert_eq!(base_char(Base::WinAnsi, 0x80), Some('\u{20AC}'));
        assert_eq!(base_char(Base::WinAnsi, 0xfc), Some('\u{fc}'));
        assert_eq!(base_char(Base::MacRoman, 0x8a), Some('\u{e4}'));
        assert_eq!(base_char(Base::Standard, 0x27), Some('\u{2019}'));
        assert_eq!(base_char(Base::Standard, 0xfb), Some('\u{df}'));
        assert_eq!(base_char(Base::Standard, 0x41), Some('A'));
        assert_eq!(base_char(Base::WinAnsi, 0x07), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn glyph_names_read_as_text() {
        assert_eq!(glyph_text("adieresis").as_deref(), Some("\u{e4}"));
        assert_eq!(glyph_text("germandbls").as_deref(), Some("\u{df}"));
        assert_eq!(glyph_text("Euro").as_deref(), Some("\u{20AC}"));
        assert_eq!(glyph_text("fi").as_deref(), Some("\u{FB01}"));
        assert_eq!(glyph_text("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(glyph_text("a.sc").as_deref(), Some("a"));
        assert_eq!(glyph_text("uni00E400FC").as_deref(), Some("\u{e4}\u{fc}"));
        assert_eq!(glyph_text("u1F600").as_deref(), Some("\u{1F600}"));
        assert_eq!(glyph_text("zero").as_deref(), Some("0"));
        assert_eq!(glyph_text("g123"), None);
    }
}

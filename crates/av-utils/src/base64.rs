// ----------------------------------------------------------------------------
// Base64 decoding, flexible to support Gmail's base64url (RFC 4648 §5, `-`/`_`)
// and standard base64 (RFC 4648 §4, `+`/`/`). Padding optional.
pub fn from_base64(text: &str) -> Vec<u8> {
    fn value(c: char) -> Option<u8> {
        match c {
            'A'..='Z' => Some(c as u8 - b'A'),
            'a'..='z' => Some(c as u8 - b'a' + 26),
            '0'..='9' => Some(c as u8 - b'0' + 52),
            '-' | '+' => Some(62),
            '_' | '/' => Some(63),
            _ => None,
        }
    }
    let values: Vec<u8> = text.chars().filter_map(value).collect();
    let mut out = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        out.push((chunk[0] << 2) | (chunk[1] >> 4));
        if chunk.len() >= 3 {
            out.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        if chunk.len() >= 4 {
            out.push((chunk[2] << 6) | chunk[3]);
        }
    }
    out
}

// ----------------------------------------------------------------------------
/// RFC 4648 §4 base64, padded (what MIME, data URLs, and JSON APIs expect).
pub fn to_base64(data: &[u8]) -> String {
    const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = encode(data, BASE64);
    while !out.len().is_multiple_of(4) {
        out.push('=');
    }
    out
}

// ----------------------------------------------------------------------------
/// RFC 4648 base64url, no padding (what PKCE and JWTs use).
pub fn encode_base64_url(data: &[u8]) -> String {
    const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    encode(data, B64_URL)
}

// ----------------------------------------------------------------------------
/// base64 encoding, no padding
fn encode(data: &[u8], table: &[u8; 64]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(table[(n >> 18) as usize & 63] as char);
        out.push(table[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(table[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(table[n as usize & 63] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn base64url_no_padding() {
        assert_eq!(encode_base64_url(b""), "");
        assert_eq!(encode_base64_url(b"f"), "Zg");
        assert_eq!(encode_base64_url(b"fo"), "Zm8");
        assert_eq!(encode_base64_url(b"foo"), "Zm9v");
        assert_eq!(
            encode_base64_url(&[0xfb, 0xff]),
            "-_8",
            "uses the URL-safe alphabet"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn base64_is_padded() {
        assert_eq!(to_base64(b""), "");
        assert_eq!(to_base64(b"f"), "Zg==");
        assert_eq!(to_base64(b"fo"), "Zm8=");
        assert_eq!(to_base64(b"foo"), "Zm9v");
        assert_eq!(
            to_base64(&[0xfb, 0xff]),
            "+/8=",
            "uses the standard alphabet"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn decode_base64_accepts_both_alphabets() {
        assert_eq!(from_base64(""), b"");
        assert_eq!(from_base64("Zg=="), b"f");
        assert_eq!(from_base64("Zg"), b"f");
        assert_eq!(from_base64("Zm8"), b"fo");
        assert_eq!(from_base64("Zm9v"), b"foo");
        assert_eq!(from_base64("Zm9vYmFy"), b"foobar");
        assert_eq!(from_base64("-_8"), [0xfb, 0xff], "base64url");
        assert_eq!(from_base64("+/8"), [0xfb, 0xff], "standard");
        assert_eq!(from_base64("Z"), b"");
        assert_eq!(from_base64("Zg!?"), b"f");
    }
}

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
}

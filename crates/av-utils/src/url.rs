// ----------------------------------------------------------------------------
/// Percent-encode everything but RFC 3986 unreserved characters, for URL
/// path segments and query values (e.g. calendar ids are email addresses).
pub fn encode_url_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ----------------------------------------------------------------------------
/// Decode a query value (`application/x-www-form-urlencoded`: `%XX` escapes,
/// `+` for space). Malformed escapes are kept verbatim.
pub fn decode_url_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn encode_and_decode_url_components() {
        assert_eq!(
            encode_url_component("user@example.com"),
            "user%40example.com"
        );
        assert_eq!(encode_url_component("a b~c"), "a%20b~c");
        assert_eq!(decode_url_component("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(decode_url_component("100%"), "100%", "a dangling % stays");
        let s = "ä/ü?x=1&y";
        assert_eq!(decode_url_component(&encode_url_component(s)), s);
    }
}

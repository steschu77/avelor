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

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn encode_url_components() {
        assert_eq!(
            encode_url_component("user@example.com"),
            "user%40example.com"
        );
        assert_eq!(encode_url_component("a b~c"), "a%20b~c");
    }
}

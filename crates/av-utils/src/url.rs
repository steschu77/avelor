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

// ----------------------------------------------------------------------------
/// The links that may open outside the app: web, email and phone.
const LINK_SCHEMES: [&str; 4] = ["http://", "https://", "mailto:", "tel:"];

/// Whether `url` starts with one of `LINK_SCHEMES`, in any case.
pub fn is_link(url: &str) -> bool {
    LINK_SCHEMES.iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    })
}

// ----------------------------------------------------------------------------
// ----------------------------------------------------------------------------
/// `url` without its query and fragment, as a log names a link: they can
/// carry what the link is about (a search, a token).
pub fn without_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
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

    // ------------------------------------------------------------------------
    #[test]
    fn a_logged_link_keeps_no_query() {
        assert_eq!(
            without_query("https://a.example/b?q=x#f"),
            "https://a.example/b"
        );
        assert_eq!(
            without_query("https://a.example/b#f?x"),
            "https://a.example/b"
        );
        assert_eq!(
            without_query("mailto:a@b.example?subject=Hi"),
            "mailto:a@b.example"
        );
        assert_eq!(without_query("https://a.example/"), "https://a.example/");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn links_are_web_email_and_phone() {
        for url in [
            "https://a.example",
            "HTTP://a.example",
            "mailto:a@b",
            "tel:+49",
        ] {
            assert!(is_link(url), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "file:///c:/x",
            "id:01a0",
            "a.md",
            "ä",
            "",
        ] {
            assert!(!is_link(url), "{url}");
        }
    }
}

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

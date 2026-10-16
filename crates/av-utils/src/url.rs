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

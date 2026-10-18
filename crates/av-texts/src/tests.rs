use super::*;

fn catalog(language: &str, json: &str) -> Catalog {
    Catalog::parse(language, json).unwrap()
}

fn used(page: &[&str], host: &[&str]) -> Used {
    Used {
        page: page.iter().map(|key| key.to_string()).collect(),
        host: host.iter().map(|key| key.to_string()).collect(),
    }
}

const EN: &str =
    r#"{"a.plain": "Dismiss", "a.count": {"one": "{n} email", "other": "{n} emails"}}"#;

// ----------------------------------------------------------------------------
#[test]
fn a_catalog_holds_texts_and_a_counts_forms() {
    let en = catalog("en", EN);
    assert_eq!(en.plain("a.plain"), Some("Dismiss"));
    assert_eq!(en.plain("a.count"), None);
    let Some(Text::Forms(forms)) = en.texts.get("a.count") else {
        panic!("no forms");
    };
    assert_eq!(forms["other"], "{n} emails");
}

// ----------------------------------------------------------------------------
#[test]
fn a_text_neither_plain_nor_forms_is_refused() {
    let refused = |json| Catalog::parse("en", json).unwrap_err();
    assert!(refused(r#"{"a": 1}"#).contains("\"a\""));
    assert!(refused(r#"{"a": {"one": "x"}}"#).contains("no \"other\""));
    assert!(refused(r#"{"a": {"several": "x", "other": "y"}}"#).contains("\"several\""));
    assert!(refused(r#"{"a": {"other": 2}}"#).contains("isn't a text"));
    assert!(refused("[]").contains("not an object"));
}

// ----------------------------------------------------------------------------
#[test]
fn a_language_over_english_fills_in_what_it_lacks() {
    let fr = catalog("fr", r#"{"a.plain": "Ignorer"}"#).over(&catalog("en", EN));
    assert_eq!(fr.plain("a.plain"), Some("Ignorer"));
    assert!(fr.texts.contains_key("a.count"));
    assert_eq!(fr.language, "fr");
}

// ----------------------------------------------------------------------------
#[test]
fn the_page_reads_a_catalog_as_one_object() {
    let json = catalog("en", EN).to_json();
    let back = catalog("en", &json);
    assert_eq!(back.texts, catalog("en", EN).texts);
}

// ----------------------------------------------------------------------------
#[test]
fn complete_catalogs_have_no_problems() {
    let de = catalog(
        "de",
        r#"{"a.plain": "Ausblenden", "a.count": {"one": "{n} E-Mail", "other": "{n} E-Mails"}}"#,
    );
    let problems = check(
        &catalog("en", EN),
        &[de],
        &["de"],
        &used(&["a.count"], &["a.plain"]),
    );
    assert_eq!(problems, Vec::<String>::new());
}

// ----------------------------------------------------------------------------
#[test]
fn an_own_catalog_lacking_a_key_fails() {
    let de = catalog("de", r#"{"a.plain": "Ausblenden"}"#);
    let problems = check(
        &catalog("en", EN),
        &[de],
        &["de"],
        &used(&["a.plain", "a.count"], &[]),
    );
    assert_eq!(problems, ["de.json: no \"a.count\", which en.json has"]);
}

// ----------------------------------------------------------------------------
#[test]
fn a_contributed_catalog_may_lack_keys_but_none_english_lacks() {
    let fr = catalog("fr", r#"{"a.plain": "Ignorer", "a.gone": "Parti"}"#);
    let problems = check(
        &catalog("en", EN),
        &[fr],
        &["de"],
        &used(&["a.plain", "a.count"], &[]),
    );
    assert_eq!(problems, ["fr.json: \"a.gone\" isn't in en.json"]);
}

// ----------------------------------------------------------------------------
#[test]
fn a_placeholder_english_lacks_fails() {
    let de = catalog(
        "de",
        r#"{"a.plain": "{who} ausblenden", "a.count": "{n} E-Mails"}"#,
    );
    let problems = check(
        &catalog("en", EN),
        &[de],
        &["de"],
        &used(&["a.plain", "a.count"], &[]),
    );
    assert_eq!(
        problems,
        ["de.json: \"a.plain\" has {who}, which en.json's hasn't"]
    );
}

// ----------------------------------------------------------------------------
#[test]
fn a_key_used_but_missing_and_one_used_nowhere_fail() {
    let problems = check(
        &catalog("en", EN),
        &[],
        &[],
        &used(&["a.plain", "a.new"], &[]),
    );
    assert_eq!(
        problems,
        [
            "en.json: no \"a.new\", which is used",
            "en.json: \"a.count\" is used nowhere"
        ]
    );
}

// ----------------------------------------------------------------------------
#[test]
fn the_host_shows_plain_texts_only() {
    let problems = check(
        &catalog("en", EN),
        &[],
        &[],
        &used(&["a.plain"], &["a.count"]),
    );
    assert_eq!(
        problems,
        ["en.json: \"a.count\" has a count's forms, but the host shows it as it is"]
    );
}

// ----------------------------------------------------------------------------
#[test]
fn the_page_uses_the_keys_its_calls_write_out() {
    let js = "const a = t('status.dismiss');\n\
              el.title = `${t(\"status.more\", { n })} and more`;\n\
              body.append(...parts('scans.read_by', { picker }));\n\
              // t('in.a.comment')\n\
              const s = 't(\\'in.a.string\\')';";
    let keys = page_keys(js).unwrap();
    assert_eq!(
        keys.into_iter().collect::<Vec<_>>(),
        ["scans.read_by", "status.dismiss", "status.more"]
    );
}

// ----------------------------------------------------------------------------
#[test]
fn a_key_not_written_out_fails() {
    let problems = page_keys("const key = 'a'; t(key);").unwrap_err();
    assert!(problems[0].contains("isn't written out"), "{problems:?}");
}

// ----------------------------------------------------------------------------
#[test]
fn the_host_uses_the_keys_it_looks_up_and_those_it_gives_the_page() {
    let rust = "let text = texts::text(\"start.failed\");\n\
                // texts::text(\"in.a.comment\")\n\
                show(texts::text(\"a.b\"));\n\
                app.notice(Text::new(\"e.f\").arg(\"n\", 2));";
    let used = host_keys(rust);
    assert_eq!(
        used.host.into_iter().collect::<Vec<_>>(),
        ["a.b", "start.failed"]
    );
    assert_eq!(used.page.into_iter().collect::<Vec<_>>(), ["e.f"]);
}

// ----------------------------------------------------------------------------
#[test]
fn the_page_s_markup_names_its_texts_by_key() {
    let html = r#"<button data-table-only data-t-title="editor.table" data-t="editor.table_label"></button>
<input data-t-placeholder="words.placeholder">"#;
    let keys = html_keys(html).unwrap();
    assert_eq!(
        keys.into_iter().collect::<Vec<_>>(),
        ["editor.table", "editor.table_label", "words.placeholder"]
    );
    let problems = html_keys(r#"<span data-t="lens.bank">Bank</span>"#).unwrap_err();
    assert!(problems[0].contains("lens.bank"), "{problems:?}");
}

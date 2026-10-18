use super::*;
use av_vault::envelope::Source;
use serde_json::json;

use super::import::{ScanPage, scan_document};
use super::read::{
    PageEvent, PageStatus, ReadCounts, TITLE_LEN, clean_ocr, is_image, title_of, word_count,
};
use crate::ai;
use crate::testing::Harness;
use crate::testing::{FakeOllama, parse, reply, shape, shown, words};
use serde_json::Value;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WATCH_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";
const IMPORTED_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b41";
const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b42";

// ------------------------------------------------------------------------
/// A harness whose vault has an images watch on the folder `scans`
/// beside it, which holds `a.jpg`, `b.jpg` and `c.jpg`; `c.jpg` is in the
/// vault already. And that folder.
fn with_scans(name: &str) -> (Harness, PathBuf) {
    with_scans_and(name, &[])
}

// ------------------------------------------------------------------------
/// `with_scans`, with the vault files `more` too (an AI account).
fn with_scans_and(name: &str, more: &[(&str, String)]) -> (Harness, PathBuf) {
    let mut h = Harness::new(name);
    let root = h.vault();
    let folder = root.with_file_name("scans");
    fs::create_dir_all(&folder).unwrap();
    for file in ["a.jpg", "b.jpg", "c.jpg"] {
        fs::write(folder.join(file), file).unwrap();
    }
    let watch = format!(
        "id: {WATCH_ID}\nfolder-watch:\n  type: images\n  folder: '{}'\n  patterns: ['*.jpg']\n  models:\n    read:\n      account: {OLLAMA_ID}\n      model: m\n",
        folder.display()
    );
    let imported = format!(
        "---\nid: {IMPORTED_ID}\ntitle: Old\ndocument: {{}}\nscan:\n  watch: {WATCH_ID}\n  origin: c.jpg\n---\nOld\n"
    );
    let files = [
        (".avelor/watches/watch_images.yaml", watch),
        ("Scans/old.md", imported),
    ];
    for (path, text) in files.into_iter().chain(more.iter().cloned()) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    crate::api::switch_to(&mut h.app, root.clone());
    h.wait_indexed();
    h.take();
    (h, folder)
}

// ------------------------------------------------------------------------
/// Keep `text` as what the file `name` of `folder` read to.
fn read_page(h: &mut Harness, folder: &Path, name: &str, text: &str) {
    let file = FolderFile::read(folder, name).unwrap();
    let ocr = Ocr {
        stamp: Stamp::of(&file),
        words: word_count(text),
        title: title_of(text),
        text: text.to_string(),
    };
    h.app.scans.cache.insert(file.path, ocr);
}

// ------------------------------------------------------------------------
#[test]
fn pages_become_one_document() {
    let watch = Id::generate();
    let first = DateTime::from_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 30));
    let later = DateTime::from_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 31));
    let sub = Path::new("2026")
        .join("scan_0806.jpg")
        .to_string_lossy()
        .into_owned();
    let page = |title| ScanPage {
        file: &sub,
        taken: first,
        text: "# Bill",
        title,
    };
    let second = ScanPage {
        file: "scan_0807.jpg",
        taken: later,
        text: "Page two",
        title: "Page two",
    };
    let pages = [page("Bill"), second];

    let doc = scan_document(watch, "  ", &pages);
    assert_eq!(doc.title(), "Bill", "the first page's title");
    assert_eq!(doc.date(), Some(first));
    assert_eq!(doc.content(), "# Bill\n\nPage two\n");
    assert!(doc.roles().document.is_some());
    let Some(Source::Scan(scan)) = doc.source() else {
        panic!("{:?}", doc.source());
    };
    assert_eq!(scan.watch, Some(watch));
    assert_eq!(
        scan.origin.as_slice(),
        [sub.clone(), "scan_0807.jpg".into()]
    );

    let doc = scan_document(watch, " Taxes ", &pages);
    assert_eq!(doc.title(), "Taxes", "the title given, trimmed");
    let doc = scan_document(watch, "", &[page("")]);
    assert_eq!(doc.title(), "scan_0806", "the first file's name");
}

// ------------------------------------------------------------------------
#[test]
fn import_writes_read_pages_and_says_why_not_the_others() {
    let (mut h, folder) = with_scans("scan-import");
    read_page(&mut h, &folder, "a.jpg", "# Bill 4711\n\nPay now.");
    read_page(&mut h, &folder, "c.jpg", "# Old");
    let docs = json!([
        { "title": "", "files": ["a.jpg"] },
        { "title": "B", "files": ["b.jpg"] },
        { "files": ["c.jpg"] },
        { "files": [] },
    ]);
    let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
    assert_eq!(shape(&sent), ["entity", "reply"]);
    let entity = &sent[0]["data"];
    assert_eq!(entity["title"], "Bill 4711");
    let reply = &sent[1]["ok"];
    assert_eq!(
        reply["imported"],
        json!([{ "files": ["a.jpg"], "id": entity["id"] }])
    );
    let failed: Vec<String> = reply["failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| shown(&f["error"]))
        .collect();
    assert_eq!(
        failed,
        [
            "b.jpg isn't read, or changed since; read it again.",
            "c.jpg is in the vault already.",
            "A document needs a page.",
        ]
    );
    let path = h.vault().join(entity["path"].as_str().unwrap());
    let text = fs::read_to_string(path).unwrap();
    assert!(
        text.contains(&format!("watch: {WATCH_ID}")) && text.contains("origin: a.jpg"),
        "{text}"
    );
    assert!(text.ends_with("# Bill 4711\n\nPay now.\n"), "{text}");

    let again = json!({ "watch": WATCH_ID, "docs": [{ "files": ["a.jpg"] }] });
    let sent = h.call("scan_import", again);
    assert_eq!(
        shown(&sent[0]["ok"]["failed"][0]["error"]),
        "a.jpg is in the vault already."
    );
}

// ------------------------------------------------------------------------
#[test]
fn the_listing_tells_what_is_read_and_what_is_in_the_vault() {
    let (mut h, folder) = with_scans("scan-list");
    read_page(&mut h, &folder, "a.jpg", "# A");
    let sent = h.call_waiting("scan_list", json!({ "watch": WATCH_ID }));
    let listing = &reply(&sent)["ok"];
    let files = listing["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["file"].as_str().unwrap()).collect();
    assert_eq!(names, ["a.jpg", "b.jpg", "c.jpg"]);
    assert_eq!(files[0]["read"]["title"], "A");
    assert_eq!(
        (&files[0]["in_vault"], &files[1]["read"]),
        (&Value::Null, &Value::Null)
    );
    assert_eq!(files[2]["in_vault"], IMPORTED_ID);
    assert_eq!(listing["job"], Value::Null, "nothing is being read");
}

// ------------------------------------------------------------------------
#[test]
fn a_read_stops_before_its_next_page_at_the_spend_limit() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let address = FakeOllama::new(&["m"])
        .replying(&["# A"])
        .recording(&requests)
        .start();
    let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
    let (mut h, _) = with_scans_and("scan-limit", &[(".avelor/accounts/ollama.yaml", account)]);
    let args = json!({ "watch": WATCH_ID, "files": ["a.jpg", "b.jpg"] });
    let request = json!({ "id": 1, "cmd": "scan_read", "args": args });
    h.app.on_web_message(request.to_string());
    // The first page is on its way; then the limit is reached.
    let id = Id::parse(OLLAMA_ID).unwrap();
    h.app.ai.update(id, |s| s.spend_limit = Some(0.0)).unwrap();
    let ended = |t: &String| t.contains(r#""state":"done""#);
    let sent = parse(&h.wait_for(|sent| sent.iter().any(ended)));

    let last_of = |file: &str| {
        let mut pages = sent.iter().filter(|m| m["ev"] == "scan_page");
        pages.rfind(|m| m["data"]["file"] == file).unwrap()["data"]["status"].clone()
    };
    assert_eq!(last_of("a.jpg"), "read");
    assert_eq!(last_of("b.jpg"), "stopped");
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["options"]["num_ctx"], ai::CONTEXT_TOKENS);
    let notice = sent.iter().find(|m| m["ev"] == "notice").unwrap();
    let message = shown(&notice["data"]["text"]);
    assert!(
        message.starts_with("Ollama has reached its spend limit"),
        "{message}"
    );
    let job = sent.iter().rfind(|m| m["ev"] == "job").unwrap();
    assert_eq!(
        shown(&job["data"]["summary"]),
        "1 read \u{b7} 1 stopped \u{b7} spend limit reached"
    );
    assert!(h.app.scans.read.is_none());

    // The next read is refused before it starts.
    let sent = h.call(
        "scan_read",
        json!({ "watch": WATCH_ID, "files": ["b.jpg"] }),
    );
    assert_eq!(sent[0]["err"]["code"], "spend_limit");
}

// ------------------------------------------------------------------------
#[test]
fn requests_for_unknown_watches_accounts_and_files_are_refused() {
    let (mut h, folder) = with_scans("scan-refused");
    let unknown = Id::generate();
    let requests = [
        ("scan_list", json!({ "watch": unknown })),
        ("scan_read", json!({ "watch": unknown, "files": [] })),
        ("scan_previews", json!({ "watch": unknown, "files": [] })),
        ("scan_open", json!({ "watch": unknown, "file": "a.jpg" })),
        ("scan_import", json!({ "watch": unknown, "docs": [] })),
    ];
    for (cmd, args) in requests {
        let sent = h.call(cmd, args);
        assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
    }

    // The account of the watch's OCR model is gone.
    let sent = h.call(
        "scan_read",
        json!({ "watch": WATCH_ID, "files": ["a.jpg"] }),
    );
    assert_eq!(shape(&sent), ["reply"], "nothing queued");
    assert_eq!(sent[0]["err"]["code"], "invalid_argument");
    assert_eq!(
        shown(&sent[0]["err"]["text"]),
        "For OCR, the picked model\u{2019}s account is gone; pick another model."
    );
    assert!(h.app.scans.read.is_none());

    fs::write(folder.with_file_name("outside.jpg"), "x").unwrap();
    let outside = Path::new("..").join("outside.jpg");
    let sent = h.call("scan_open", json!({ "watch": WATCH_ID, "file": outside }));
    assert_eq!(sent[0]["err"]["code"], "invalid_path");
    let sent = h.call(
        "scan_open",
        json!({ "watch": WATCH_ID, "file": "gone.jpg" }),
    );
    assert_eq!(sent[0]["err"]["code"], "invalid_path");
}

// ------------------------------------------------------------------------
#[test]
fn answers_lose_a_fence_around_them() {
    assert_eq!(clean_ocr("  # Bill\n\nText\n "), "# Bill\n\nText");
    assert_eq!(
        clean_ocr("```markdown\n# Bill\n\nText\n```\n"),
        "# Bill\n\nText"
    );
    assert_eq!(clean_ocr("```\nText\n```"), "Text");
    let code = "```\nfn a()\n```\nAfter the code.";
    assert_eq!(clean_ocr(code), code, "a fenced block at the start stays");
    assert_eq!(clean_ocr(" \n"), "");
}

// ------------------------------------------------------------------------
#[test]
fn titles_come_from_the_first_heading_or_line() {
    let letter = "Stadtwerke Springfield\nMain Street 1\n\n## Rechnung Nr. 4711\n\nText";
    assert_eq!(title_of(letter), "Rechnung Nr. 4711");
    assert_eq!(title_of("\n**Dear Mr. Yahnek,**\nText"), "Dear Mr. Yahnek,");
    assert_eq!(title_of("| Date | Amount |\n|---|---|"), "Date Amount");
    let long = format!("# {}", "word ".repeat(30));
    assert!(title_of(&long).chars().count() <= TITLE_LEN + 1);
    assert_eq!(title_of(""), "");
}

// ------------------------------------------------------------------------
#[test]
fn words_are_runs_with_a_letter_or_digit() {
    assert_eq!(word_count("# Bill\n\n- 3 x 4,50 EUR\n| a | b |"), 7);
    assert_eq!(word_count(""), 0);
}

// ------------------------------------------------------------------------
#[test]
fn statuses_and_summaries_read_well() {
    fn json(status: PageStatus) -> String {
        serde_json::to_string(&status).unwrap()
    }
    assert_eq!(json(PageStatus::Queued), r#"{"status":"queued"}"#);
    let watch = Id::generate();
    let gone = Text::new("reader.unread");
    let status = PageStatus::Failed { error: &gone };
    let event = PageEvent {
        watch,
        file: "a.jpg",
        status: &status,
    };
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!({ "watch": watch, "file": "a.jpg", "status": "failed", "error": { "key": "reader.unread" } })
    );
    assert_eq!(
        json(PageStatus::Read {
            words: 3,
            title: "Bill"
        }),
        r#"{"status":"read","words":3,"title":"Bill"}"#
    );
    let mut counts = ReadCounts {
        read: 2,
        failed: 0,
        stopped: 1,
        limit: false,
    };
    assert_eq!(words(&counts.summary()), "2 read · 1 stopped");
    counts.limit = true;
    assert_eq!(
        words(&counts.summary()),
        "2 read · 1 stopped · spend limit reached"
    );
    assert_eq!(words(&ReadCounts::default().summary()), "nothing to read");
    assert!(is_image("scan.JPG") && is_image("p1.webp") && !is_image("a.pdf"));
}

// ------------------------------------------------------------------------
#[test]
fn only_files_in_the_folder_open_and_stamps_tell_changes() {
    let dir = std::env::temp_dir().join(format!("avelor-scan-test-{}", Id::generate()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("a.jpg"), "a").unwrap();
    std::fs::write(dir.with_extension("outside"), "x").unwrap();
    let file = FolderFile::read(&dir, "sub/a.jpg").unwrap();
    let stamp = Stamp::of(&file);
    let in_sub = inside(&dir, &dir.join("sub").join("a.jpg"));
    let up = inside(
        &dir.join("sub"),
        &dir.join("sub").join("..").join("sub").join("a.jpg"),
    );
    let escaped = inside(
        &dir.join("sub"),
        &dir.join("sub").join("..").join("..").join("x"),
    );
    let folder = inside(&dir, &dir.join("sub"));
    std::fs::write(dir.join("sub").join("a.jpg"), "changed").unwrap();
    let changed = FolderFile::read(&dir, "sub/a.jpg").unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_file(dir.with_extension("outside")).unwrap();

    assert!(in_sub && up, "inside, also by way of ..");
    assert!(!escaped && !folder, "nothing outside, no folders");
    assert!(stamp.matches(&file) && !stamp.matches(&changed));
}

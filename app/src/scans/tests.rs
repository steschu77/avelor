use super::*;
use crate::testing::{Harness, reply};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

const WATCH_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";
const IMPORTED_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b41";

// ------------------------------------------------------------------------
/// A harness whose vault has an images watch on the folder `scans`
/// beside it, which holds `a.jpg`, `b.jpg` and `c.jpg`; `c.jpg` is in the
/// vault already. And that folder.
fn with_scans(name: &str) -> (Harness, PathBuf) {
    let mut h = Harness::new(name);
    let root = h.vault();
    let folder = root.with_file_name("scans");
    fs::create_dir_all(&folder).unwrap();
    for file in ["a.jpg", "b.jpg", "c.jpg"] {
        fs::write(folder.join(file), file).unwrap();
    }
    let watch = format!(
        "id: {WATCH_ID}\nfolder-watch:\n  type: images\n  folder: '{}'\n  patterns: ['*.jpg']\n",
        folder.display()
    );
    let imported = format!(
        "---\nid: {IMPORTED_ID}\ntitle: Old\ndocument: {{}}\nscan:\n  watch: {WATCH_ID}\n  origin: c.jpg\n---\nOld\n"
    );
    let files = [
        (".avelor/watches/watch_images.yaml", watch),
        ("Scans/old.md", imported),
    ];
    for (path, text) in files {
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
#[test]
fn the_listing_tells_what_is_in_the_vault() {
    let (mut h, _folder) = with_scans("scan-list");
    let sent = h.call_waiting("scan_list", json!({ "watch": WATCH_ID }));
    let listing = &reply(&sent)["ok"];
    let files = listing["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["file"].as_str().unwrap()).collect();
    assert_eq!(names, ["a.jpg", "b.jpg", "c.jpg"]);
    assert_eq!(files[0]["in_vault"], Value::Null);
    assert_eq!(files[2]["in_vault"], IMPORTED_ID);
}

// ------------------------------------------------------------------------
#[test]
fn requests_for_unknown_watches_are_refused() {
    let (mut h, _folder) = with_scans("scan-refused");
    let unknown = Id::generate();
    let requests = [("scan_list", json!({ "watch": unknown }))];
    for (cmd, args) in requests {
        let sent = h.call(cmd, args);
        assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
    }
}

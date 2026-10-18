use super::*;
use serde_json::json;

use crate::testing::{Harness, reply};
use serde_json::Value;
use std::fs;

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
fn requests_for_unknown_watches_and_files_are_refused() {
    let (mut h, folder) = with_scans("scan-refused");
    let unknown = Id::generate();
    let requests = [
        ("scan_list", json!({ "watch": unknown })),
        ("scan_previews", json!({ "watch": unknown, "files": [] })),
        ("scan_open", json!({ "watch": unknown, "file": "a.jpg" })),
    ];
    for (cmd, args) in requests {
        let sent = h.call(cmd, args);
        assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
    }

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

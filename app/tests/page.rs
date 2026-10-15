// The page's tests (development.md#checks): each tests/page/*.test.js,
// bundled with the modules it imports as the page is, run by node in a few
// time zones. Without node they are left out, but not in CI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// The page's layers (build.rs), the tests above them.
const LAYERS: &[&str] = &["static/core/", "tests/page/"];

// Ahead of UTC, behind it, and with its summer in December.
const ZONES: [&str; 3] = ["Europe/Berlin", "America/Chicago", "Pacific/Auckland"];

#[test]
fn the_pages_tests_pass() {
    if !has_node() {
        return;
    }
    let mut files: Vec<String> = fs::read_dir(app().join("tests/page"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".test.js"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "there are no tests");
    let failed: Vec<String> = files
        .iter()
        .filter_map(|file| run(file, &read(file)).err())
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn a_file_that_fails_is_told() {
    if !has_node() {
        return;
    }
    let file = |body: &str| format!("import {{ equal, test }} from './harness.js';\n{body}\n");
    let cases = [
        ("test('a', () => equal(1, 1));", None),
        ("test('a', () => equal(1, 2));", Some("FAIL a")),
        (
            "test('a', () => { throw new Error('no'); });",
            Some("Error: no"),
        ),
        (
            "test('a', async () => { await null; equal(1, 2); });",
            Some("FAIL a"),
        ),
        (
            "test('a', () => new Promise(() => {}));",
            Some("it never ends"),
        ),
        ("", Some("0 of 0 passed")),
        ("equal(1, 1); gone();", Some("gone")),
        (
            "import { gone } from '../../static/core/rpc.js';\ngone();",
            Some("gone"),
        ),
    ];
    for (body, problem) in cases {
        match (run("case.test.js", &file(body)), problem) {
            (Ok(()), None) => {}
            (Err(told), Some(problem)) => assert!(told.contains(problem), "{body}: {told}"),
            (found, _) => panic!("{body}: {found:?}"),
        }
    }
}

// ----------------------------------------------------------------------------
// Whether node is there; CI has it.
fn has_node() -> bool {
    let found = Command::new("node").arg("--version").output().is_ok();
    assert!(
        found || std::env::var_os("CI").is_none(),
        "node isn't installed"
    );
    if !found {
        eprintln!("node isn't installed: the page's tests were left out");
    }
    found
}

fn app() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(file: &str) -> String {
    fs::read_to_string(app().join("tests/page").join(file)).unwrap()
}

// ----------------------------------------------------------------------------
// Runs the test file `name`, its text `source`, in each of `ZONES`; what
// failed, if anything did.
fn run(name: &str, source: &str) -> Result<(), String> {
    let entry = format!("tests/page/{name}");
    let bundle = av_bundle::bundle(&entry, LAYERS, |path| {
        if path == entry {
            return Ok(source.to_string());
        }
        fs::read_to_string(app().join(path)).map_err(|e| e.to_string())
    })
    .map_err(|problems| format!("{entry}:\n{}", problems.join("\n")))?;

    let script: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::write(&script, bundle.js).unwrap();
    for zone in ZONES {
        let out = Command::new("node")
            .arg(&script)
            .env("TZ", zone)
            .output()
            .unwrap();
        if !out.status.success() {
            return Err(format!(
                "{entry} in {zone}:\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    Ok(())
}

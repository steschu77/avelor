// The docs' references (architecture.md, Doc conventions): in the docs, the
// README and the code's comments a `name.md#id` names an id at the end of a
// heading of docs/name.md; in a doc a `#id` alone a section of that doc, or
// else an element of the page; a file name in backticks a file of the
// repository, a link its file. Each id is short, unique in its doc and
// referred to; the docs are LF.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const SOURCES: [&str; 4] = [".rs", ".js", ".css", ".html"];

// this file's own cases name docs and ids that don't exist
const THIS: &str = "app/tests/docs.rs";

type Ids = BTreeMap<String, usize>; // an id -> its line

#[test]
fn the_docs_references_hold() {
    let found = problems(&Path::new(env!("CARGO_MANIFEST_DIR")).join(".."));
    assert!(
        found.is_empty(),
        "the docs' references:\n{}",
        found.join("\n")
    );
}

#[test]
fn each_rule_is_found() {
    let root = std::env::temp_dir().join(format!("avelor-docs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let a = "# A <a id=\"top\"></a>\n\n## Used <a id=\"used\"></a>\n\n## Unused <a id=\"unused\"></a>\n\n\
             ## Long <a id=\"much-too-long-an-id\"></a>\n\n## Again <a id=\"used\"></a>\n\n\
             Not at a heading's end <a id=\"loose\"></a>.\n\n\
             `#used`, `#main`, `#nothing`, `b.md#b`, `b.md#gone`, `c.md#x`, `b.md`, `c.md`, `lib.rs`, \
             `src/lib.rs:3`, `gone.rs`, [up](#top).\n\n```text\n`#ignored`, b.md#ignored\n```\n";
    let files = [
        (
            "README.md",
            "See [A](docs/a.md#top), [B](docs/b.md#b) and [this](docs/gone.md).\n",
        ),
        ("docs/a.md", a),
        (
            "docs/b.md",
            "# B <a id=\"b\"></a>\r\n\r\nSee `a.md#top`.\r\n",
        ),
        ("app/static/index.html", "<div id=\"main\"></div>\n"),
        ("app/src/lib.rs", "// a.md#used, a.md#gone\n"),
    ];
    for (file, text) in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let found = problems(&root);
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        found,
        [
            "docs/a.md:7: `much-too-long-an-id` is no id: lowercase letters, digits and hyphens, at most 16",
            "docs/a.md:9: the id `used` is on line 3 already",
            "docs/a.md:11: an id away from its heading's end",
            "docs/a.md:13: `#nothing` is no section of a.md and no element of the page",
            "docs/a.md:13: no id `gone` in b.md",
            "docs/a.md:13: no doc c.md",
            "docs/a.md:13: no doc c.md",
            "docs/a.md:13: `gone.rs` names no file of the repository",
            "docs/b.md: CR line endings, the docs are LF",
            "README.md:1: no file docs/gone.md",
            "app/src/lib.rs:1: no id `gone` in a.md",
            "docs/a.md:7: the id `much-too-long-an-id` is referred to nowhere",
            "docs/a.md:5: the id `unused` is referred to nowhere",
        ]
    );
}

// What the docs under `root` get wrong, a line each (`docs/panel.md:108: ...`).
fn problems(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk(root, "", &mut files);
    files.sort();
    let mut out = Vec::new();
    let mut ids = BTreeMap::new();
    let mut docs = Vec::new();
    for file in &files {
        let Some(name) = file
            .strip_prefix("docs/")
            .filter(|name| name.ends_with(".md") && !name.contains('/'))
        else {
            continue;
        };
        let text = read(root, file);
        ids.insert(name.to_string(), anchors(file, &text, &mut out));
        docs.push((file.clone(), Some(name.to_string()), text));
    }
    if files.iter().any(|file| file == "README.md") {
        docs.push(("README.md".to_string(), None, read(root, "README.md")));
    }
    let elements = elements(root, &files);
    let mut check = Check {
        root,
        files,
        ids,
        elements,
        used: BTreeSet::new(),
        out,
    };
    for (file, own, text) in &docs {
        check.doc(file, own.as_deref(), text);
    }

    let code: Vec<String> = check
        .files
        .iter()
        .filter(|file| is_code(file))
        .cloned()
        .collect();
    for file in &code {
        let text = read(root, file);
        let starts = line_starts(&text);
        for (offset, name, id) in doc_refs(&text) {
            check.reference(&format!("{file}:{}", line_at(&starts, offset)), name, id);
        }
    }

    for (name, ids) in &check.ids {
        for (id, line) in ids {
            if !check.used.contains(&(name.clone(), id.clone())) {
                check.out.push(format!(
                    "docs/{name}:{line}: the id `{id}` is referred to nowhere"
                ));
            }
        }
    }
    check.out
}

struct Check<'a> {
    root: &'a Path,
    files: Vec<String>,
    ids: BTreeMap<String, Ids>, // of docs/<name>, by name
    elements: BTreeSet<String>,
    used: BTreeSet<(String, String)>, // the doc's name and the id
    out: Vec<String>,
}

impl Check<'_> {
    // A doc (`own` its name) or the README.
    fn doc(&mut self, file: &str, own: Option<&str>, text: &str) {
        if text.contains('\r') {
            self.out
                .push(format!("{file}: CR line endings, the docs are LF"));
        }
        let starts = line_starts(text);
        let mut in_block = false;
        for (event, range) in parse(text) {
            let at = format!("{file}:{}", line_at(&starts, range.start));
            match event {
                Event::Start(Tag::CodeBlock(_)) => in_block = true,
                Event::End(TagEnd::CodeBlock) => in_block = false,
                Event::Text(words) if !in_block => {
                    for (_, name, id) in doc_refs(&words) {
                        self.reference(&at, name, id);
                    }
                }
                Event::Code(code) => self.span(&at, own, &code),
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                    self.link(&at, file, &dest_url);
                }
                _ => {}
            }
        }
    }

    fn reference(&mut self, at: &str, name: &str, id: &str) {
        match self.ids.get(name) {
            None => self.out.push(format!("{at}: no doc {name}")),
            Some(ids) if !ids.contains_key(id) => {
                self.out.push(format!("{at}: no id `{id}` in {name}"))
            }
            Some(_) => {
                self.used.insert((name.to_string(), id.to_string()));
            }
        }
    }

    // A code span: a `#id` alone, a file name, a doc's name, or text that may
    // hold references.
    fn span(&mut self, at: &str, own: Option<&str>, code: &str) {
        if let Some(id) = code.strip_prefix('#').filter(|id| is_word(id)) {
            if let Some(doc) = own.filter(|doc| self.ids[*doc].contains_key(id)) {
                self.used.insert((doc.to_string(), id.to_string()));
            } else if !self.elements.contains(id) {
                let section = own.map_or(String::new(), |doc| format!("no section of {doc} and "));
                self.out
                    .push(format!("{at}: `#{id}` is {section}no element of the page"));
            }
        } else if let Some(path) = source_path(code) {
            if !self.has_file(path) {
                self.out
                    .push(format!("{at}: `{code}` names no file of the repository"));
            }
        } else if code
            .strip_suffix(".md")
            .is_some_and(|name| !name.is_empty() && name.bytes().all(is_name_byte))
        {
            if !self.ids.contains_key(code) && !self.files.iter().any(|file| file == code) {
                self.out.push(format!("{at}: no doc {code}"));
            }
        } else {
            for (_, name, id) in doc_refs(code) {
                self.reference(at, name, id);
            }
        }
    }

    fn link(&mut self, at: &str, file: &str, dest: &str) {
        if dest.contains("://") || dest.starts_with("mailto:") {
            return;
        }
        let (path, id) = dest
            .split_once('#')
            .map_or((dest, None), |(path, id)| (path, Some(id)));
        let target = if path.is_empty() {
            file.to_string()
        } else {
            joined(file, path)
        };
        if !self.root.join(&target).exists() {
            self.out.push(format!("{at}: no file {target}"));
        } else if let Some(id) = id {
            match target
                .strip_prefix("docs/")
                .filter(|name| self.ids.contains_key(*name))
            {
                Some(name) => self.reference(at, name, id),
                None => self
                    .out
                    .push(format!("{at}: {target} is no doc, for `#{id}`")),
            }
        }
    }

    // `path` is the end of a file's path at a `/`, `src/` left out.
    fn has_file(&self, path: &str) -> bool {
        let tail = format!("/{path}");
        self.files
            .iter()
            .filter(|file| is_source(file))
            .any(|file| {
                let short = file.replace("/src/", "/");
                file == path || file.ends_with(&tail) || short == path || short.ends_with(&tail)
            })
    }
}

// The ids at the end of `file`'s headings; one anywhere else is a problem.
fn anchors(file: &str, text: &str, out: &mut Vec<String>) -> Ids {
    let starts = line_starts(text);
    let mut ids = Ids::new();
    let mut heading: Option<Vec<(String, usize)>> = None; // its events, the HTML ones by their text
    for (event, range) in parse(text) {
        let line = line_at(&starts, range.start);
        match event {
            Event::Start(Tag::Heading { .. }) => heading = Some(Vec::new()),
            Event::End(TagEnd::Heading(_)) => {
                let parts = heading.take().unwrap_or_default();
                for (i, (html, line)) in parts.iter().enumerate() {
                    let Some(id) = anchor(html) else { continue };
                    if i + 2 != parts.len() || parts[i + 1].0 != "</a>" {
                        out.push(format!("{file}:{line}: an id away from its heading's end"));
                        continue;
                    }
                    if !is_id(id) {
                        out.push(format!(
                            "{file}:{line}: `{id}` is no id: lowercase letters, digits and hyphens, at most 16"
                        ));
                    }
                    match ids.get(id) {
                        Some(first) => out.push(format!(
                            "{file}:{line}: the id `{id}` is on line {first} already"
                        )),
                        None => {
                            ids.insert(id.to_string(), *line);
                        }
                    }
                }
            }
            Event::InlineHtml(html) | Event::Html(html) => match &mut heading {
                Some(parts) => parts.push((html.to_string(), line)),
                None if html.contains("<a id=") => {
                    out.push(format!("{file}:{line}: an id away from its heading's end"))
                }
                None => {}
            },
            _ => {
                if let Some(parts) = &mut heading {
                    parts.push((String::new(), line));
                }
            }
        }
    }
    ids
}

fn anchor(html: &str) -> Option<&str> {
    html.strip_prefix("<a id=\"")?.strip_suffix("\">")
}

// The page's element ids: `id="..."` in its HTML, `.id = '...'` in a module.
fn elements(root: &Path, files: &[String]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for file in files.iter().filter(|file| file.starts_with("app/static/")) {
        let (marker, close) = if file.ends_with(".html") {
            (" id=\"", '"')
        } else if file.ends_with(".js") {
            (".id = '", '\'')
        } else {
            continue;
        };
        let text = read(root, file);
        for (i, _) in text.match_indices(marker) {
            let rest = &text[i + marker.len()..];
            if let Some(end) = rest.find(close) {
                ids.insert(rest[..end].to_string());
            }
        }
    }
    ids
}

// Each `name.md#id` in `text`, with its offset.
fn doc_refs(text: &str) -> Vec<(usize, &str, &str)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(".md#") {
        let dot = from + i;
        let mut start = dot;
        while start > 0 && is_name_byte(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = dot + 4;
        while end < bytes.len() && is_word_byte(bytes[end]) {
            end += 1;
        }
        if start < dot && dot + 4 < end {
            found.push((start, &text[start..dot + 3], &text[dot + 4..end]));
        }
        from = dot + 4;
    }
    found
}

// A code span naming a source file, with a `:line` or `:from-to` or not.
fn source_path(code: &str) -> Option<&str> {
    let path = match code.split_once(':') {
        None => code,
        Some((path, lines))
            if lines
                .split('-')
                .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            path
        }
        Some(_) => return None,
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    let plain = path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b));
    (plain
        && SOURCES
            .iter()
            .any(|ext| name.len() > ext.len() && name.ends_with(ext)))
    .then_some(path)
}

// `path`, relative to `file`'s folder, from the root.
fn joined(file: &str, path: &str) -> String {
    let mut parts: Vec<&str> = file.split('/').collect();
    parts.pop();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn is_source(file: &str) -> bool {
    SOURCES.iter().any(|ext| file.ends_with(ext))
}

fn is_code(file: &str) -> bool {
    (file.starts_with("app/") || file.starts_with("crates/")) && is_source(file) && file != THIS
}

fn is_id(id: &str) -> bool {
    id.len() <= 16 && id.starts_with(|c: char| c.is_ascii_lowercase()) && is_word(id)
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(is_word_byte)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn parse(text: &str) -> pulldown_cmark::OffsetIter<'_, pulldown_cmark::DefaultBrokenLinkCallback> {
    Parser::new_ext(text, Options::ENABLE_TABLES).into_offset_iter()
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

fn line_at(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&start| start <= offset)
}

// Every file under `dir` but the build's and git's, `/`-separated.
fn walk(root: &Path, dir: &str, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}/{name}")
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                if name != "target" && name != ".git" {
                    walk(root, &path, files);
                }
            }
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }
}

fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
}

use oxc::{
    allocator::Allocator,
    codegen::{Codegen, CodegenOptions, CommentOptions},
    minifier::{Minifier, MinifierOptions},
    parser::Parser,
    span::SourceType,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
// ----------------------------------------------------------------------------
/// The page's layers, lowest first: a module in one of these folders imports
/// only from its own layer or a lower one, and every module but an entry is
/// in one (`av_bundle::bundle`, `architecture.md#page`).
const LAYERS: &[&str] = &["core/", "editor/", "model/", "ui/", "features/"];

// ----------------------------------------------------------------------------
/// The page source `path` under `static_dir`. The sources are ASCII-only,
/// with escapes for anything else (`architecture.md#page`): a non-ASCII
/// character is an error that names its line and column.
fn read_source(static_dir: &Path, path: &str) -> Result<String, String> {
    let text = fs::read_to_string(static_dir.join(path)).map_err(|e| e.to_string())?;
    for (row, line) in text.lines().enumerate() {
        if let Some((col, c)) = line.chars().enumerate().find(|(_, c)| !c.is_ascii()) {
            return Err(format!(
                "line {}, column {}: {c:?} (U+{:04X}) isn't ASCII; write it as an escape \
                 (architecture.md#page)",
                row + 1,
                col + 1,
                c as u32
            ));
        }
    }
    Ok(text)
}

/// `read_source` for a file the page can't do without: a problem fails the
/// build.
fn source(static_dir: &Path, path: &str) -> String {
    read_source(static_dir, path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// ----------------------------------------------------------------------------
/// The bundle of the module `entry` under `static_dir` and the modules it
/// imports; a problem fails the build, each one listed.
fn bundle(static_dir: &Path, entry: &str) -> av_bundle::Bundle {
    av_bundle::bundle(entry, LAYERS, |path| read_source(static_dir, path)).unwrap_or_else(
        |problems| {
            for problem in &problems {
                eprintln!("{problem}");
            }
            panic!("the page's modules failed to bundle (development.md#the-bundled-page)");
        },
    )
}

// ----------------------------------------------------------------------------
/// `html` with its module script `tag` replaced by `js`, inlined. A
/// `</script` in `js` (in a string) would end the inline script early, so it
/// becomes `<\/script`, which means the same in a string.
fn inline_script(html: &str, tag: &str, js: &str) -> String {
    assert!(html.contains(tag), "the page has no {tag}");
    let js = js.replace("</script", r"<\/script");
    html.replace(tag, &format!("<script>\n{js}\n</script>"))
}

// ----------------------------------------------------------------------------
fn process_javascript(source: &str) -> String {
    let allocator = Allocator::default();

    // The bundle is a classic script, as the page includes it: parsed as a
    // module, the minifier drops 'use strict' and mangles top-level names on
    // the assumption that block-level functions stay in their blocks.
    let result = Parser::new(&allocator, source, SourceType::script()).parse();

    if !result.diagnostics.is_empty() {
        eprintln!("JavaScript syntax errors:");

        for diagnostic in &result.diagnostics {
            eprintln!("{diagnostic:?}");
        }

        panic!("JavaScript syntax check failed");
    }

    if result.fatal_error {
        panic!("JavaScript parser encountered a fatal error");
    }

    if env::var("PROFILE").as_deref() == Ok("release") {
        let mut program = result.program;

        let minifier = Minifier::new(MinifierOptions::default()).minify(&allocator, &mut program);

        Codegen::new()
            .with_options(CodegenOptions {
                minify: true,
                comments: CommentOptions::disabled(),
                ..Default::default()
            })
            .with_scoping(minifier.scoping)
            .build(&program)
            .code
    } else {
        source.to_owned()
    }
}

// ----------------------------------------------------------------------------
/// Minify `css` without changing what it means: comments are dropped,
/// whitespace collapses to one space and goes where it can't matter -- after
/// `{ } ; , > :` and before `{ } ; , >` -- and the `;` before a `}` goes.
/// Before `:` it stays (`a :hover` is a descendant combinator), and so it
/// does around `+ - *` (`calc(a + b)`). Strings and `\` escapes are copied
/// as they are.
fn minify_css(css: &str) -> String {
    const NO_SPACE_AFTER: &str = "{};,>:";
    const NO_SPACE_BEFORE: &str = "{};,>";

    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let mut space = false;

    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for c in chars.by_ref() {
                if prev == '*' && c == '/' {
                    break;
                }
                prev = c;
            }
            continue;
        }

        if c.is_whitespace() {
            space = true;
            continue;
        }

        let tight = out
            .chars()
            .next_back()
            .is_none_or(|prev| NO_SPACE_AFTER.contains(prev));
        if space && !tight && !NO_SPACE_BEFORE.contains(c) {
            out.push(' ');
        }
        space = false;

        if c == '}' && out.ends_with(';') {
            out.pop();
        }
        out.push(c);

        match c {
            '"' | '\'' => {
                while let Some(s) = chars.next() {
                    out.push(s);
                    if s == '\\' {
                        out.extend(chars.next());
                    } else if s == c {
                        break;
                    }
                }
            }
            '\\' => out.extend(chars.next()),
            _ => {}
        }
    }

    out
}

// ----------------------------------------------------------------------------
fn process_css(source: &str) -> String {
    if env::var("PROFILE").as_deref() == Ok("release") {
        minify_css(source)
    } else {
        source.to_owned()
    }
}

// ----------------------------------------------------------------------------
/// `html` with its stylesheet links replaced by one inlined `<style>` where
/// the first one was, with the CSS of the linked files in the order they
/// are linked: the cascade order (`architecture.md#css`).
fn inline_styles(static_dir: &Path, html: &str) -> String {
    let href_of = |line: &str| {
        line.trim()
            .strip_prefix("<link rel=\"stylesheet\" href=\"")?
            .strip_suffix("\">")
            .map(str::to_owned)
    };
    let mut css = String::new();
    let mut page = String::new();
    let mut style_at = None;
    for line in html.split_inclusive('\n') {
        match href_of(line) {
            Some(href) => {
                css.push_str(&source(static_dir, &href));
                style_at.get_or_insert(page.len());
            }
            None => page.push_str(line),
        }
    }
    let at = style_at.expect("index.html links no stylesheet");
    page.insert_str(at, &format!("<style>\n{}\n</style>\n", process_css(&css)));
    page
}

// ----------------------------------------------------------------------------
/// The icons, `icons/*.svg` under `static_dir` (Fluent UI System Icons), as
/// one hidden SVG of symbols, each `#i-<file name>`: the file's `viewBox`
/// and shapes, their `fill` left to the page's CSS (`.icon`). An icon that
/// none of `sources` refers to, or a reference to an icon that isn't
/// there, fails the build.
fn icon_sprite(static_dir: &Path, sources: &[&str]) -> String {
    let mut files: Vec<PathBuf> = fs::read_dir(static_dir.join("icons"))
        .expect("app/static has no icons folder")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "svg"))
        .collect();
    files.sort();

    let mut names = Vec::new();
    let mut symbols = String::new();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let file = format!("icons/{name}.svg");
        let svg = source(static_dir, &file);
        let start = svg
            .find("<svg")
            .unwrap_or_else(|| panic!("{file}: no <svg>"));
        let (tag, rest) = svg[start..]
            .split_once('>')
            .unwrap_or_else(|| panic!("{file}: no end of <svg>"));
        let view_box = tag
            .split_once("viewBox=\"")
            .and_then(|(_, after)| after.split_once('"'))
            .map(|(view_box, _)| view_box)
            .unwrap_or_else(|| panic!("{file}: no viewBox"));
        let (shapes, _) = rest
            .rsplit_once("</svg>")
            .unwrap_or_else(|| panic!("{file}: no </svg>"));
        let shapes = without_fill(shapes.trim());
        symbols.push_str(&format!(
            "<symbol id=\"i-{name}\" viewBox=\"{view_box}\">{shapes}</symbol>\n"
        ));
        names.push(name);
    }

    let used: Vec<String> = sources.iter().flat_map(|text| icon_refs(text)).collect();
    for name in &names {
        assert!(
            used.contains(name),
            "icons/{name}.svg: no page source refers to #i-{name}"
        );
    }
    for name in &used {
        assert!(
            names.contains(name),
            "#i-{name}: there is no icons/{name}.svg"
        );
    }
    format!("<svg style=\"display: none\" aria-hidden=\"true\">\n{symbols}</svg>\n")
}

// ----------------------------------------------------------------------------
/// `svg` without its `fill` attributes.
fn without_fill(svg: &str) -> String {
    let mut out = String::new();
    let mut rest = svg;
    while let Some(at) = rest.find(" fill=\"") {
        out.push_str(&rest[..at]);
        let value = &rest[at + " fill=\"".len()..];
        rest = value.split_once('"').map_or("", |(_, after)| after);
    }
    out.push_str(rest);
    out
}

// ----------------------------------------------------------------------------
/// The icons `text` refers to: the names after `#i-`.
fn icon_refs(text: &str) -> impl Iterator<Item = String> + '_ {
    text.match_indices("#i-")
        .map(|(at, _)| {
            text[at + "#i-".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
}

// ----------------------------------------------------------------------------
/// The page, `index.html` with its CSS, its icons and the bundle of `app.js`
/// inlined, and what the bundle's modules import (`page-deps.txt`); and the
/// markdown selftest with its own bundle (`markdown.md#testing`), which
/// opens from disk. All are written to `out_dir`.
fn build_pages(root: &Path, out_dir: &Path) {
    let static_dir = root.join("static");

    let index = source(&static_dir, "index.html");
    let html = inline_styles(&static_dir, &index);

    let page = bundle(&static_dir, "app.js");
    fs::write(out_dir.join("page-deps.txt"), page.graph.to_string()).unwrap();
    let sprite = icon_sprite(&static_dir, &[&index, &page.js]);
    assert!(html.contains("<body>\n"), "index.html has no <body>");
    let html = html.replacen("<body>\n", &format!("<body>\n{sprite}"), 1);
    let html = inline_script(
        &html,
        "<script type=\"module\" src=\"./app.js\"></script>",
        &process_javascript(&page.js),
    );
    fs::write(out_dir.join("index.html"), html).unwrap();

    let selftest = bundle(&static_dir, "editor/markdown.selftest.js");
    let html = source(&static_dir, "editor/markdown.selftest.html");
    let html = inline_script(
        &html,
        "<script type=\"module\" src=\"./markdown.selftest.js\"></script>",
        &process_javascript(&selftest.js),
    );
    fs::write(out_dir.join("markdown.selftest.html"), html).unwrap();

    println!("cargo:rerun-if-changed=static");
}

// ----------------------------------------------------------------------------
fn main() {
    println!("cargo:rerun-if-changed=avelor.rc");
    println!("cargo:rerun-if-changed=res");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    build_pages(&root, &out_dir);

    let _ = embed_resource::compile("avelor.rc", embed_resource::NONE);
    static_vcruntime::metabuild();
}

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
const LAYERS: &[&str] = &["core/", "model/", "ui/", "features/"];

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
    av_bundle::bundle(entry, LAYERS, |path| read_source(static_dir, path))
        .unwrap_or_else(|problems| bundle_failed(&problems))
}

// ----------------------------------------------------------------------------
/// The page's modules' `problems`, listed; the build fails.
fn bundle_failed(problems: &[String]) -> ! {
    for problem in problems {
        eprintln!("{problem}");
    }
    panic!("the page's modules failed to bundle (development.md#the-bundled-page)");
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
/// The lines of `css` where a declaration's value has a color of its own: a
/// hex color, a color function, `white` or `black`. Only the tokens hold
/// colors, each with its dark value. A selector's `:` starts what looks like
/// a value too, but its `{` drops what was found in it (`#add-btn`).
fn color_literals(css: &str) -> Vec<usize> {
    const FUNCTIONS: &[&str] = &[
        "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color",
    ];
    const NAMES: &[&str] = &["white", "black"];
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let chars: Vec<char> = css.chars().collect();
    let mut lines = Vec::new();
    let mut found = Vec::new();
    let mut line = 1;
    let mut in_value = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let skip_to = match c {
            '/' if chars.get(i + 1) == Some(&'*') => (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
                .map(|j| j + 2),
            '"' | '\'' => (i + 1..chars.len())
                .find(|&j| chars[j] == c && chars[j - 1] != '\\')
                .map(|j| j + 1),
            _ => None,
        };
        if let Some(end) = skip_to {
            line += chars[i..end].iter().filter(|&&c| c == '\n').count();
            i = end;
            continue;
        }
        match c {
            '\n' => line += 1,
            ':' => in_value = true,
            ';' | '}' => {
                lines.append(&mut found);
                in_value = false;
            }
            '{' => {
                found.clear();
                in_value = false;
            }
            '#' if in_value => {
                let digits = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_hexdigit())
                    .count();
                let after = chars.get(i + 1 + digits).copied();
                if [3, 4, 6, 8].contains(&digits) && !after.is_some_and(ident) {
                    found.push(line);
                }
            }
            c if in_value && c.is_ascii_alphabetic() && (i == 0 || !ident(chars[i - 1])) => {
                let len = chars[i..].iter().take_while(|&&c| ident(c)).count();
                let word: String = chars[i..i + len].iter().collect();
                let call = chars.get(i + len) == Some(&'(');
                if NAMES.contains(&word.as_str()) || call && FUNCTIONS.contains(&word.as_str()) {
                    found.push(line);
                }
                i += len;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    lines
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
                let text = source(static_dir, &href);
                if href != "css/tokens.css" {
                    let lines = color_literals(&text);
                    assert!(
                        lines.is_empty(),
                        "{href}: a color of its own on line {lines:?}; colors are the tokens' \
                         (architecture.md#css)"
                    );
                }
                css.push_str(&text);
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
/// The page, `index.html` with its CSS and the bundle of `app.js` inlined,
/// and what the bundle's modules import (`page-deps.txt`), written to
/// `out_dir`.
/// An export no module of the bundle imports fails the build.
fn build_pages(root: &Path, out_dir: &Path) {
    let static_dir = root.join("static");

    let html = source(&static_dir, "index.html");
    let html = inline_styles(&static_dir, &html);

    let page = bundle(&static_dir, "app.js");
    fs::write(out_dir.join("page-deps.txt"), page.graph.to_string()).unwrap();
    let html = inline_script(
        &html,
        "<script type=\"module\" src=\"./app.js\"></script>",
        &process_javascript(&page.js),
    );
    fs::write(out_dir.join("index.html"), html).unwrap();

    let unused = av_bundle::unused_exports(&[&page.graph]);
    if !unused.is_empty() {
        bundle_failed(&unused);
    }
    println!("cargo:rerun-if-changed=static");
}

// ----------------------------------------------------------------------------
/// The manifest's version (`res/avelor.exe.manifest`, `assemblyIdentity`),
/// which the resource compiler takes as it is: the workspace's with `.0`,
/// or the build fails.
fn check_manifest(root: &Path) {
    let path = root.join("res/avelor.exe.manifest");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let identity = text
        .split_once("<assemblyIdentity")
        .and_then(|(_, rest)| rest.split_once("/>"))
        .map(|(identity, _)| identity)
        .expect("res/avelor.exe.manifest has no assemblyIdentity");
    let version = identity
        .split_once("version=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(version, _)| version)
        .expect("res/avelor.exe.manifest's assemblyIdentity has no version");
    let want = format!("{}.0", env::var("CARGO_PKG_VERSION").unwrap());
    assert!(
        version == want,
        "res/avelor.exe.manifest: assemblyIdentity's version is {version}, not {want}, the \
         workspace's version"
    );
}

// ----------------------------------------------------------------------------
/// The resource compiler's macros for `avelor.rc`'s VERSIONINFO: the
/// version's numbers, and its text, which the compiler can't make of them.
fn version_macros() -> Vec<String> {
    let part = |name: &str| env::var(format!("CARGO_PKG_VERSION_{name}")).unwrap();
    vec![
        format!("VERSION_MAJOR={}", part("MAJOR")),
        format!("VERSION_MINOR={}", part("MINOR")),
        format!("VERSION_PATCH={}", part("PATCH")),
        format!(
            "VERSION_TEXT=\"{}\"",
            env::var("CARGO_PKG_VERSION").unwrap()
        ),
    ]
}

// ----------------------------------------------------------------------------
fn main() {
    println!("cargo:rerun-if-changed=avelor.rc");
    println!("cargo:rerun-if-changed=res");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    build_pages(&root, &out_dir);
    check_manifest(&root);

    let _ = embed_resource::compile("avelor.rc", version_macros());
    static_vcruntime::metabuild();
}

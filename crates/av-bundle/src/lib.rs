//! The page's ES modules bundled into the one classic script the page
//! loads (`development.md#the-bundled-page`): the modules the entry imports, in
//! the order browsers run them, each a function scope that returns its exports,
//! an import a `const` taken from them. The bundle also checks what the
//! modules may do: no import cycle; named imports and exports only, and
//! imports for a module's side effects; no exported `let`; no write to an
//! import; no name used that another module declares without importing it;
//! and the page's layers. Build-time only.

use oxc::{
    allocator::Allocator,
    ast::ast::{Declaration, ImportDeclarationSpecifier, Statement},
    parser::Parser,
    semantic::SemanticBuilder,
    span::{GetSpan, SourceType, Span},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

// ----------------------------------------------------------------------------
/// The bundle of an entry module.
pub struct Bundle {
    /// The modules in the order they run, with what each imports.
    pub graph: Graph,
    /// One classic script in strict mode: the modules in the order they run.
    pub js: String,
}

// ----------------------------------------------------------------------------
/// A module in the bundle.
pub struct File {
    /// Its path under the page's folder, like `core/dom.js`.
    pub path: String,
    /// The names it imports, by the index of the module they come from; none
    /// for a module it imports for its side effects only.
    pub imports: BTreeMap<usize, BTreeSet<String>>,
}

// ----------------------------------------------------------------------------
/// The modules of a bundle in the order they run.
pub struct Graph {
    pub files: Vec<File>,
}

// ----------------------------------------------------------------------------
/// An import declaration: the module it names, and the names it takes as
/// `(imported, local)`; none for an import of the module's side effects.
struct Import {
    specifier: String,
    names: Vec<(String, String)>,
}

// ----------------------------------------------------------------------------
/// What the bundle needs of a parsed module.
struct Module {
    path: String,
    source: String,
    imports: Vec<Import>,
    /// The module each import resolved to, once loaded.
    targets: Vec<Option<usize>>,
    /// `(exported, local)`.
    exports: Vec<(String, String)>,
    /// The names it declares at its top level, imports aside.
    declares: BTreeSet<String>,
    /// The names it uses but doesn't declare: the browser's (`document`,
    /// `Map`, ...), or a forgotten import.
    free: BTreeSet<String>,
    /// The byte ranges the bundle leaves out: the import declarations and
    /// the `export` keywords.
    cuts: Vec<(usize, usize)>,
}

// ----------------------------------------------------------------------------
/// The bundle of `entry` and the modules it imports, read by `read` by their
/// paths under the page's folder. A module in one of the `layers` (folders
/// like `core/`, lowest first) imports only from its own layer or a lower
/// one; every module but the entry is in a layer. Fails with every problem
/// found, each naming its modules.
pub fn bundle(
    entry: &str,
    layers: &[&str],
    read: impl Fn(&str) -> Result<String, String>,
) -> Result<Bundle, Vec<String>> {
    let mut loader = Loader {
        read: &read,
        modules: Vec::new(),
        index: HashMap::new(),
        done: Vec::new(),
        order: Vec::new(),
        problems: Vec::new(),
    };
    loader.visit(entry, None, &mut Vec::new());
    if !loader.problems.is_empty() {
        return Err(loader.problems);
    }

    let Loader { modules, order, .. } = loader;
    let problems = check(&modules, &order, entry, layers);
    if !problems.is_empty() {
        return Err(problems);
    }

    let position: HashMap<usize, usize> = order.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let files = order
        .iter()
        .map(|&i| {
            let module = &modules[i];
            let mut imports = BTreeMap::<usize, BTreeSet<String>>::new();
            for (import, target) in module.imports.iter().zip(&module.targets) {
                imports
                    .entry(position[&target.unwrap()])
                    .or_default()
                    .extend(import.names.iter().map(|(imported, _)| imported.clone()));
            }
            File {
                path: module.path.clone(),
                imports,
            }
        })
        .collect();

    Ok(Bundle {
        graph: Graph { files },
        js: emit(&modules, &order, entry),
    })
}

// ----------------------------------------------------------------------------
/// Loads the modules depth-first from the entry, each after the modules it
/// imports, in the order of its imports: the order browsers run them.
struct Loader<'r, R> {
    read: &'r R,
    modules: Vec<Module>,
    index: HashMap<String, usize>,
    done: Vec<bool>,
    order: Vec<usize>,
    problems: Vec<String>,
}

impl<R: Fn(&str) -> Result<String, String>> Loader<'_, R> {
    /// The module at `path`, loaded with its imports; `stack` holds the
    /// modules being loaded, to name a cycle.
    fn visit(&mut self, path: &str, from: Option<&str>, stack: &mut Vec<usize>) -> Option<usize> {
        if let Some(&i) = self.index.get(path) {
            if !self.done[i] {
                let at = stack.iter().position(|&j| j == i).unwrap();
                let cycle: Vec<_> = stack[at..]
                    .iter()
                    .map(|&j| self.modules[j].path.as_str())
                    .chain([path])
                    .collect();
                self.problems
                    .push(format!("an import cycle: {}", cycle.join(" -> ")));
            }
            return Some(i);
        }

        let source = match (self.read)(path) {
            Ok(source) => source,
            Err(e) => {
                self.problems.push(match from {
                    Some(from) => format!("{from} imports {path}, which can't be read: {e}"),
                    None => format!("{path} can't be read: {e}"),
                });
                return None;
            }
        };
        let module = parse(path, source, &mut self.problems);
        let specifiers: Vec<_> = module.imports.iter().map(|i| i.specifier.clone()).collect();

        let i = self.modules.len();
        self.index.insert(path.to_string(), i);
        self.modules.push(module);
        self.done.push(false);
        stack.push(i);
        let targets = specifiers
            .iter()
            .map(|specifier| match resolve(path, specifier) {
                Ok(target) => self.visit(&target, Some(path), stack),
                Err(e) => {
                    self.problems
                        .push(format!("{path} imports '{specifier}': {e}"));
                    None
                }
            })
            .collect();
        stack.pop();
        self.modules[i].targets = targets;
        self.done[i] = true;
        self.order.push(i);
        Some(i)
    }
}

// ----------------------------------------------------------------------------
/// The path `specifier` names, imported by the module at `from`: relative,
/// a `.js` file, and inside the page's folder.
fn resolve(from: &str, specifier: &str) -> Result<String, String> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) || !specifier.ends_with(".js")
    {
        return Err("an import names a .js file by its relative path".into());
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in specifier.split('/') {
        match part {
            "." => {}
            ".." => {
                parts.pop().ok_or("the path leaves the page's folder")?;
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

// ----------------------------------------------------------------------------
/// A byte range from `span`, with the line break after it.
fn line_cut(source: &str, span: Span) -> (usize, usize) {
    let (start, mut end) = (span.start as usize, span.end as usize);
    let rest = &source[end..];
    end += if rest.starts_with("\r\n") {
        2
    } else {
        usize::from(rest.starts_with('\n'))
    };
    (start, end)
}

// ----------------------------------------------------------------------------
/// What the bundle needs of the module `source` at `path`; what it can't
/// bundle goes to `problems`.
fn parse(path: &str, source: String, problems: &mut Vec<String>) -> Module {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &source, SourceType::mjs()).parse();
    for diagnostic in &parsed.diagnostics {
        problems.push(format!("{path}: {diagnostic:?}"));
    }
    if parsed.fatal_error {
        problems.push(format!("{path}: the parser gave up"));
    }
    if !parsed.module_record.dynamic_imports.is_empty() {
        problems.push(format!("{path} uses import(), which doesn't bundle"));
    }
    if !parsed.module_record.import_metas.is_empty() {
        problems.push(format!("{path} uses import.meta, which doesn't bundle"));
    }

    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut cuts = Vec::new();
    for statement in &parsed.program.body {
        match statement {
            Statement::ImportDeclaration(decl) => {
                let mut names = Vec::new();
                for specifier in decl.specifiers.iter().flatten() {
                    match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            names.push((s.imported.name().to_string(), s.local.name.to_string()))
                        }
                        _ => problems.push(format!(
                            "{path} has a default or namespace import from {}: only named imports bundle",
                            decl.source.value
                        )),
                    }
                }
                imports.push(Import {
                    specifier: decl.source.value.to_string(),
                    names,
                });
                cuts.push(line_cut(&source, decl.span));
            }
            Statement::ExportDeclaration(decl) => {
                let declaration = &decl.declaration;
                cuts.push((decl.span.start as usize, declaration.span().start as usize));
                let names: Vec<String> = match declaration {
                    Declaration::VariableDeclaration(v) => v
                        .declarations
                        .iter()
                        .flat_map(|d| d.id.get_binding_identifiers())
                        .map(|id| id.name.to_string())
                        .collect(),
                    Declaration::FunctionDeclaration(f) => {
                        f.id.iter().map(|id| id.name.to_string()).collect()
                    }
                    Declaration::ClassDeclaration(c) => {
                        c.id.iter().map(|id| id.name.to_string()).collect()
                    }
                    _ => Vec::new(),
                };
                exports.extend(names.into_iter().map(|name| (name.clone(), name)));
            }
            Statement::ExportNamedDeclaration(decl) => {
                for s in &decl.specifiers {
                    exports.push((s.exported.name().to_string(), s.local.name().to_string()));
                }
                cuts.push(line_cut(&source, decl.span));
            }
            Statement::ExportDefaultDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportFromDeclaration(_) => problems.push(format!(
                "{path} has an export default, an export * or a re-export: only named exports of its own names bundle"
            )),
            _ => {}
        }
    }

    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let scoping = semantic.scoping();
    let bindings = scoping.get_bindings(scoping.root_scope_id());
    let mut declares = BTreeSet::new();
    for (name, &symbol) in bindings {
        if !scoping.symbol_flags(symbol).is_import() {
            declares.insert(name.to_string());
        } else if scoping
            .get_resolved_references(symbol)
            .any(|reference| reference.is_write())
        {
            problems.push(format!(
                "{path} assigns to {name}, an import: a module changes its state through its own functions"
            ));
        }
    }
    for (exported, local) in &exports {
        let flags = bindings
            .get(local.as_str())
            .map(|&symbol| scoping.symbol_flags(symbol));
        if flags.is_some_and(|f| !(f.is_const_variable() || f.is_function() || f.is_class())) {
            problems.push(format!(
                "{path} exports {exported}, which isn't a const, a function or a class: \
                 without live bindings, a module changes its state through its own functions"
            ));
        }
    }
    let free = scoping
        .root_unresolved_references()
        .keys()
        .map(|name| name.to_string())
        .collect();

    Module {
        path: path.to_string(),
        imports,
        targets: Vec::new(),
        exports,
        declares,
        free,
        cuts,
        source,
    }
}

// ----------------------------------------------------------------------------
/// The name the bundle gives the exports of the module at `path`:
/// `__core_dom` for `core/dom.js`.
fn var_name(path: &str) -> String {
    let stem = path.strip_suffix(".js").unwrap_or(path);
    format!(
        "__{}",
        stem.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    )
}

// ----------------------------------------------------------------------------
/// What breaks the rules among the loaded `modules`: names imported that
/// aren't exported, a name used without its import, the layers, a module
/// name the bundle gives.
fn check(modules: &[Module], order: &[usize], entry: &str, layers: &[&str]) -> Vec<String> {
    let layer_of = |path: &str| layers.iter().position(|layer| path.starts_with(layer));
    let mut problems = Vec::new();

    let mut declared_by = HashMap::new();
    for &i in order {
        for name in &modules[i].declares {
            declared_by.entry(name.as_str()).or_insert(i);
        }
    }
    let mut vars = HashMap::new();
    for &i in order {
        if let Some(j) = vars.insert(var_name(&modules[i].path), i) {
            problems.push(format!(
                "{} and {} get the same name in the bundle",
                modules[j].path, modules[i].path
            ));
        }
    }

    for &i in order {
        let module = &modules[i];
        let own = layer_of(&module.path);
        if own.is_none() && module.path != entry {
            problems.push(format!(
                "{} is in no layer (architecture.md#page)",
                module.path
            ));
        }

        for (import, target) in module.imports.iter().zip(&module.targets) {
            let other = &modules[target.unwrap()];
            let names: Vec<_> = import
                .names
                .iter()
                .map(|(imported, _)| imported.as_str())
                .collect();
            if let (Some(own), Some(theirs)) = (own, layer_of(&other.path)) {
                if theirs > own {
                    let what = if names.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", names.join(", "))
                    };
                    problems.push(format!(
                        "{} imports{what} from {}, a higher layer ({})",
                        module.path, other.path, layers[theirs]
                    ));
                }
            }
            for name in names {
                if !other.exports.iter().any(|(exported, _)| exported == name) {
                    problems.push(format!(
                        "{} imports {name} from {}, which doesn't export it",
                        module.path, other.path
                    ));
                }
            }
        }

        for name in &module.free {
            if let Some(&j) = declared_by.get(name.as_str()) {
                problems.push(format!(
                    "{} uses {name}, which {} declares, without importing it",
                    module.path, modules[j].path
                ));
            }
        }
        for name in module.declares.iter().chain(&module.free) {
            if vars.contains_key(name) {
                problems.push(format!(
                    "{} uses {name}, the name the bundle gives a module",
                    module.path
                ));
            }
        }
    }
    problems
}

// ----------------------------------------------------------------------------
/// The bundle's script: strict, each module in the order it runs as a
/// function scope that takes its imports from the modules before it and
/// returns its exports; the entry's scope returns nothing.
fn emit(modules: &[Module], order: &[usize], entry: &str) -> String {
    let mut js = String::from("'use strict';\n");
    for &i in order {
        let module = &modules[i];
        js.push_str(&format!("\n// {}\n", module.path));
        if module.path == entry {
            js.push_str("(function () {\n");
        } else {
            js.push_str(&format!(
                "const {} = (function () {{\n",
                var_name(&module.path)
            ));
        }

        for (import, target) in module.imports.iter().zip(&module.targets) {
            if import.names.is_empty() {
                continue;
            }
            let fields: Vec<_> = import
                .names
                .iter()
                .map(|(imported, local)| field(imported, local))
                .collect();
            let from = var_name(&modules[target.unwrap()].path);
            js.push_str(&format!("const {{ {} }} = {from};\n", fields.join(", ")));
        }

        let mut cuts = module.cuts.clone();
        cuts.sort_unstable();
        let mut at = 0;
        for (start, end) in cuts {
            js.push_str(&module.source[at..start]);
            at = end;
        }
        js.push_str(&module.source[at..]);
        if !js.ends_with('\n') {
            js.push('\n');
        }

        if module.path != entry {
            let fields: Vec<_> = module
                .exports
                .iter()
                .map(|(exported, local)| field(exported, local))
                .collect();
            js.push_str(&format!("return {{ {} }};\n", fields.join(", ")));
        }
        js.push_str("})();\n");
    }
    js
}

/// An object's field for `name`, taken from or given to `local`.
fn field(name: &str, local: &str) -> String {
    if name == local {
        name.to_string()
    } else {
        format!("{name}: {local}")
    }
}

// ----------------------------------------------------------------------------
impl fmt::Display for Graph {
    /// The report: a count, then each module in the order it runs, with the
    /// names it imports by the module they come from; a module imported for
    /// its side effects only stands alone.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let imports = self
            .files
            .iter()
            .map(|file| file.imports.len())
            .sum::<usize>();
        let names = self
            .files
            .iter()
            .flat_map(|file| file.imports.values().flatten())
            .collect::<BTreeSet<_>>()
            .len();
        writeln!(
            f,
            "{} modules, in the order they run; {names} names imported from another module; {imports} imports",
            self.files.len()
        )?;
        for file in &self.files {
            writeln!(f, "\n{}", file.path)?;
            for (&j, names) in &file.imports {
                let path = &self.files[j].path;
                if names.is_empty() {
                    writeln!(f, "  {path}")?;
                } else {
                    let names = names
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(f, "  {path}: {names}")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYERS: &[&str] = &["core/", "ui/"];

    fn try_bundle(files: &[(&str, &str)]) -> Result<Bundle, Vec<String>> {
        let files: HashMap<&str, &str> = files.iter().copied().collect();
        bundle("app.js", LAYERS, |path| {
            files
                .get(path)
                .map(|source| source.to_string())
                .ok_or_else(|| "no such file".to_string())
        })
    }

    fn bundled(files: &[(&str, &str)]) -> Bundle {
        try_bundle(files).unwrap_or_else(|problems| panic!("{problems:?}"))
    }

    fn problems(files: &[(&str, &str)]) -> Vec<String> {
        match try_bundle(files) {
            Ok(_) => panic!("bundled"),
            Err(problems) => problems,
        }
    }

    fn order(bundle: &Bundle) -> Vec<&str> {
        bundle
            .graph
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn modules_run_after_their_imports_in_the_order_of_the_imports() {
        let b = bundled(&[
            (
                "app.js",
                "import { menu } from './ui/menu.js';\nimport './core/log.js';\nimport { el } from './core/dom.js';\nmenu(el);",
            ),
            (
                "ui/menu.js",
                "import { el } from '../core/dom.js';\nexport function menu() { el(); }",
            ),
            ("core/dom.js", "export function el() {}"),
            ("core/log.js", "console.log('loaded');"),
        ]);
        assert_eq!(
            order(&b),
            ["core/dom.js", "ui/menu.js", "core/log.js", "app.js"]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_bundle_is_a_strict_script_with_a_scope_per_module() {
        let b = bundled(&[
            (
                "app.js",
                "import { el as make, count } from './core/dom.js';\nmake(count);",
            ),
            (
                "core/dom.js",
                "// The DOM helpers.\nexport function el() {}\nconst n = 1;\nexport { n as count };\n",
            ),
        ]);
        assert_eq!(
            b.js,
            "'use strict';\n\
             \n// core/dom.js\n\
             const __core_dom = (function () {\n\
             // The DOM helpers.\nfunction el() {}\nconst n = 1;\n\
             return { el, count: n };\n})();\n\
             \n// app.js\n\
             (function () {\n\
             const { el: make, count } = __core_dom;\n\
             make(count);\n})();\n"
        );
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &b.js, SourceType::script()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_import_cycle_fails() {
        assert_eq!(
            problems(&[
                ("app.js", "import { a } from './core/a.js';\na();"),
                (
                    "core/a.js",
                    "import { b } from './b.js';\nexport function a() { b(); }"
                ),
                (
                    "core/b.js",
                    "import { a } from './a.js';\nexport function b() { a(); }"
                ),
            ]),
            ["an import cycle: core/a.js -> core/b.js -> core/a.js"]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_import_the_module_doesnt_export_fails() {
        assert_eq!(
            problems(&[
                (
                    "app.js",
                    "import { el, gone } from './core/dom.js';\nel(gone);"
                ),
                ("core/dom.js", "export function el() {}\nfunction gone() {}"),
            ]),
            ["app.js imports gone from core/dom.js, which doesn't export it"]
        );
        assert_eq!(
            problems(&[("app.js", "import './core/gone.js';")]),
            ["app.js imports core/gone.js, which can't be read: no such file"]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_exported_let_fails() {
        let found = problems(&[
            ("app.js", "import { n } from './core/count.js';\nn;"),
            ("core/count.js", "export let n = 0;"),
        ]);
        assert_eq!(found.len(), 1);
        assert!(
            found[0].starts_with("core/count.js exports n, which isn't a const"),
            "{found:?}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_named_imports_and_exports_bundle() {
        let found = problems(&[
            (
                "app.js",
                "import * as dom from './core/dom.js';\nimport('./core/dom.js');\ndom;",
            ),
            (
                "core/dom.js",
                "export default function el() {}\nexport * from './x.js';",
            ),
        ]);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found[0].starts_with("app.js uses import()"), "{found:?}");
        assert!(
            found[1].starts_with("app.js has a default or namespace import"),
            "{found:?}"
        );
        assert!(
            found[2].starts_with("core/dom.js has an export default"),
            "{found:?}"
        );
        assert!(
            found[3].starts_with("core/dom.js has an export default"),
            "{found:?}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_write_to_an_import_fails() {
        let found = problems(&[
            ("app.js", "import { n } from './core/count.js';\nn = 2;"),
            ("core/count.js", "export const n = 0;"),
        ]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("app.js assigns to n, an import"),
            "{found:?}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_name_used_without_its_import_fails() {
        assert_eq!(
            problems(&[
                ("app.js", "import './core/dom.js';\nel(document);"),
                ("core/dom.js", "export function el() {}"),
            ]),
            ["app.js uses el, which core/dom.js declares, without importing it"]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn layers_import_only_from_their_own_layer_or_a_lower_one() {
        let menu = (
            "ui/menu.js",
            "import { el } from '../core/dom.js';\nexport function menu() { el(); }",
        );
        bundled(&[
            ("app.js", "import { menu } from './ui/menu.js';\nmenu();"),
            menu,
            ("core/dom.js", "export function el() {}"),
        ]);
        assert_eq!(
            problems(&[
                ("app.js", "import { menu } from './ui/menu.js';\nmenu();"),
                menu,
                (
                    "core/dom.js",
                    "import '../ui/log.js';\nexport function el() {}"
                ),
                ("ui/log.js", ""),
            ]),
            ["core/dom.js imports from ui/log.js, a higher layer (ui/)"]
        );
        assert_eq!(
            problems(&[
                ("app.js", "import { x } from './lens.js';\nx();"),
                ("lens.js", "export function x() {}"),
            ]),
            ["lens.js is in no layer (architecture.md#page)"]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_report_lists_the_modules_in_the_order_they_run() {
        let b = bundled(&[
            (
                "app.js",
                "import { $ } from './core/dom.js';\nimport './ui/menu.js';\n$();",
            ),
            (
                "ui/menu.js",
                "import { $, el } from '../core/dom.js';\n$(el);",
            ),
            (
                "core/dom.js",
                "export const $ = 1;\nexport function el() {}",
            ),
        ]);
        assert_eq!(
            b.graph.to_string(),
            "3 modules, in the order they run; 2 names imported from another module; 3 imports\n\
             \ncore/dom.js\n\
             \nui/menu.js\n  core/dom.js: $, el\n\
             \napp.js\n  core/dom.js: $\n  ui/menu.js\n"
        );
    }
}

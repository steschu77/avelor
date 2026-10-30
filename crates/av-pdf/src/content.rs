//! A page's content streams run for what reading needs: the text operators
//! with the fonts' glyphs placed through the matrices, form XObjects, and
//! where images are painted. Paths are only noted as painted. An error
//! inside a stream ends it with what was read so far.

use std::collections::HashSet;
use std::rc::Rc;

use crate::document::{Document, PageNode};
use crate::font::Font;
use crate::layout::{self, Placed};
use crate::lexer::{Lexer, Token, is_space};
use crate::object::{Dict, Object, Ref};

/// How deep form XObjects nest.
const MAX_FORMS: usize = 8;
/// Operators run on a page, at most.
const MAX_OPERATORS: usize = 2_000_000;
/// How much of the page an image must cover to be its scan: a scan fitted
/// to the page keeps a margin (one came to 88%).
pub const SCAN_COVER: f64 = 0.5;

// ----------------------------------------------------------------------------
/// What a page reads to.
#[derive(Debug, Default)]
pub struct Page {
    /// Its text, laid out in lines and paragraphs.
    pub text: String,
    pub letters: usize,
    pub glyphs: usize,
    /// Glyphs without text, or whose text is a control or private-use
    /// character.
    pub unmapped: usize,
    /// Nothing shown or painted on it.
    pub blank: bool,
    /// How much of the page its largest image covers, from 0 to 1.
    pub cover: f64,
    pub scan: Option<Scan>,
}

// ----------------------------------------------------------------------------
/// A page that is one JPEG image covering it.
#[derive(Debug)]
pub struct Scan {
    pub jpeg: Vec<u8>,
    /// Shown as stored: not turned or mirrored.
    pub upright: bool,
    pub cmyk: bool,
}

impl Page {
    // ------------------------------------------------------------------------
    /// Whether its text layer can stand for the page: some words, few
    /// glyphs without text.
    pub fn usable(&self) -> bool {
        self.letters >= 20 && self.unmapped * 10 <= self.glyphs
    }
}

// ----------------------------------------------------------------------------
type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

// ----------------------------------------------------------------------------
/// `m` then `n`, in PDF's row-vector order.
fn concat(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

// ----------------------------------------------------------------------------
fn point(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

// ----------------------------------------------------------------------------
/// Whether `m` draws along x and up along y, without turning or mirroring.
fn upright(m: &Matrix) -> bool {
    m[0] > 0.0 && m[3] > 0.0 && m[1].abs() <= 0.05 * m[0] && m[2].abs() <= 0.05 * m[3]
}

// ----------------------------------------------------------------------------
/// The matrix showing a page turned `turns` quarters clockwise.
fn turned(turns: u8) -> Matrix {
    match turns {
        1 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        2 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        3 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        _ => IDENTITY,
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
}

// ----------------------------------------------------------------------------
/// An image painted, where.
struct Painted {
    image: Rc<Object>,
    bounds: [f64; 4],
    upright: bool,
}

// ----------------------------------------------------------------------------
struct Run<'d, 'a> {
    doc: &'d Document<'a>,
    glyphs: Vec<Placed>,
    /// Glyphs along another direction, in the order shown.
    turned: Vec<String>,
    images: Vec<Painted>,
    painted: bool,
    letters: usize,
    count: usize,
    unmapped: usize,
    operators: usize,
}

// ----------------------------------------------------------------------------
/// The page `node` read.
pub(crate) fn read_page(doc: &Document, node: &PageNode) -> Page {
    let mut run = Run {
        doc,
        glyphs: Vec::new(),
        turned: Vec::new(),
        images: Vec::new(),
        painted: false,
        letters: 0,
        count: 0,
        unmapped: 0,
        operators: 0,
    };
    let state = State {
        ctm: turned(node.turns),
        font: None,
        size: 0.0,
        char_spacing: 0.0,
        word_spacing: 0.0,
        scale: 1.0,
        leading: 0.0,
        rise: 0.0,
    };
    let contents = doc.entry(&node.dict, "Contents");
    let parts: Vec<Rc<Object>> = match &*contents {
        Object::Array(items) => items.iter().map(|o| doc.resolve(o)).collect(),
        Object::Stream(_) => vec![Rc::clone(&contents)],
        _ => Vec::new(),
    };
    let mut data = Vec::new();
    for part in parts {
        if let Some(stream) = part.as_stream() {
            let decoded = doc.decoded(stream);
            if decoded.rest.is_none() {
                data.extend_from_slice(&decoded.data);
                data.push(b'\n');
            }
        }
    }
    let resources = doc.resolve(&node.resources);
    let resources = resources.as_dict().cloned().unwrap_or_default();
    let mut forms = HashSet::new();
    run.content(&data, &resources, state, &mut forms, 0);

    let blank = run.count == 0 && !run.painted && run.images.is_empty();
    let (cover, scan) = run.scan(&node.area, node.turns);
    let lines = layout::lines(std::mem::take(&mut run.glyphs));
    let mut text = layout::to_text(&lines);
    let turned = run.turned.concat();
    if !turned.trim().is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(turned.trim());
    }
    Page {
        text,
        letters: run.letters,
        glyphs: run.count,
        unmapped: run.unmapped,
        blank,
        cover,
        scan,
    }
}

impl Run<'_, '_> {
    // ------------------------------------------------------------------------
    /// Run the content `data` with `resources`.
    fn content(
        &mut self,
        data: &[u8],
        resources: &Dict,
        mut state: State,
        forms: &mut HashSet<Ref>,
        depth: usize,
    ) {
        let mut stack: Vec<State> = Vec::new();
        let mut text_matrix = IDENTITY;
        let mut line_matrix = IDENTITY;
        let mut operands: Vec<Object> = Vec::new();
        let mut lexer = Lexer::new(data, 0);
        loop {
            if let Some(object) = lexer.object() {
                if operands.len() < 64 {
                    operands.push(object);
                }
                continue;
            }
            let Some(token) = lexer.next_token() else {
                break;
            };
            let Token::Keyword(op) = token else {
                operands.clear();
                continue;
            };
            self.operators += 1;
            if self.operators > MAX_OPERATORS {
                break;
            }
            let num = |i: usize| operands.get(i).and_then(Object::as_f64).unwrap_or(0.0);
            match op {
                b"q" => {
                    if stack.len() < 64 {
                        stack.push(state.clone());
                    }
                }
                b"Q" => {
                    if let Some(saved) = stack.pop() {
                        state = saved;
                    }
                }
                b"cm" => {
                    let m = [num(0), num(1), num(2), num(3), num(4), num(5)];
                    state.ctm = concat(&m, &state.ctm);
                }
                b"BT" => {
                    text_matrix = IDENTITY;
                    line_matrix = IDENTITY;
                }
                b"Tf" => {
                    state.size = num(1);
                    state.font = operands
                        .first()
                        .and_then(Object::as_name)
                        .and_then(|name| self.font(resources, name));
                }
                b"Tc" => state.char_spacing = num(0),
                b"Tw" => state.word_spacing = num(0),
                b"Tz" => state.scale = num(0) / 100.0,
                b"TL" => state.leading = num(0),
                b"Ts" => state.rise = num(0),
                b"Td" | b"TD" => {
                    if op == b"TD" {
                        state.leading = -num(1);
                    }
                    line_matrix = concat(&[1.0, 0.0, 0.0, 1.0, num(0), num(1)], &line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tm" => {
                    line_matrix = [num(0), num(1), num(2), num(3), num(4), num(5)];
                    text_matrix = line_matrix;
                }
                b"T*" | b"'" | b"\"" => {
                    if op == b"\"" {
                        state.word_spacing = num(0);
                        state.char_spacing = num(1);
                    }
                    line_matrix = concat(&[1.0, 0.0, 0.0, 1.0, 0.0, -state.leading], &line_matrix);
                    text_matrix = line_matrix;
                    if op != b"T*" {
                        if let Some(Object::Str(bytes)) = operands.last() {
                            self.show(bytes, &state, &mut text_matrix);
                        }
                    }
                }
                b"Tj" => {
                    if let Some(Object::Str(bytes)) = operands.first() {
                        self.show(bytes, &state, &mut text_matrix);
                    }
                }
                b"TJ" => {
                    let items = operands.first().and_then(Object::as_array).unwrap_or(&[]);
                    for item in items {
                        match item {
                            Object::Str(bytes) => self.show(bytes, &state, &mut text_matrix),
                            _ => {
                                let shift = item.as_f64().unwrap_or(0.0);
                                let tx = -shift / 1000.0 * state.size * state.scale;
                                text_matrix = concat(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], &text_matrix);
                            }
                        }
                    }
                }
                b"Do" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        self.paint(resources, name, &state, forms, depth);
                    }
                }
                b"BI" => {
                    self.skip_inline_image(&mut lexer);
                    self.painted = true;
                }
                b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"S" | b"s" | b"sh" => {
                    self.painted = true;
                }
                _ => {}
            }
            operands.clear();
        }
    }

    // ------------------------------------------------------------------------
    /// The font `name` of `resources`, read once per font object.
    fn font(&self, resources: &Dict, name: &str) -> Option<Rc<Font>> {
        let fonts = self.doc.entry(resources, "Font");
        let entry = fonts.as_dict()?.get(name)?.clone();
        if let Object::Ref(r) = entry {
            if let Some(font) = self.doc.fonts.borrow().get(&r) {
                return Some(Rc::clone(font));
            }
            let object = self.doc.get(r.num)?;
            let font = Rc::new(Font::load(self.doc, object.as_dict()?));
            self.doc.fonts.borrow_mut().insert(r, Rc::clone(&font));
            return Some(font);
        }
        Some(Rc::new(Font::load(self.doc, entry.as_dict()?)))
    }

    // ------------------------------------------------------------------------
    /// Show the string `bytes`: each glyph placed, the text matrix moved
    /// past it.
    fn show(&mut self, bytes: &[u8], state: &State, text_matrix: &mut Matrix) {
        let Some(font) = state.font.clone() else {
            return;
        };
        for glyph in font.glyphs(bytes) {
            let at = concat(text_matrix, &state.ctm);
            let render = concat(
                &[
                    state.size * state.scale,
                    0.0,
                    0.0,
                    state.size,
                    0.0,
                    state.rise,
                ],
                &at,
            );
            let advance = glyph.width / 1000.0 * state.size
                + state.char_spacing
                + if glyph.space { state.word_spacing } else { 0.0 };
            let tx = advance * state.scale;

            self.count += 1;
            let text = glyph.text.unwrap_or_default();
            let unreadable = text.is_empty()
                || text.chars().any(|c| {
                    (c.is_control() && !c.is_whitespace())
                        || ('\u{E000}'..='\u{F8FF}').contains(&c)
                        || c == '\u{FFFD}'
                });
            if unreadable {
                self.unmapped += 1;
            } else {
                self.letters += text.chars().filter(|c| c.is_alphabetic()).count();
                let (x, y) = point(&render, 0.0, 0.0);
                let size = (render[2] * render[2] + render[3] * render[3]).sqrt();
                if upright(&render) {
                    let width = glyph.width / 1000.0 * state.size * state.scale * at[0];
                    self.glyphs.push(Placed {
                        x,
                        end: x + width,
                        baseline: -y,
                        size,
                        text,
                    });
                } else if size > 0.0 {
                    self.turned.push(text);
                }
            }
            *text_matrix = concat(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], text_matrix);
        }
    }

    // ------------------------------------------------------------------------
    /// `Do`: a form run with its resources and matrix, an image noted where
    /// it is painted.
    fn paint(
        &mut self,
        resources: &Dict,
        name: &str,
        state: &State,
        forms: &mut HashSet<Ref>,
        depth: usize,
    ) {
        let xobjects = self.doc.entry(resources, "XObject");
        let Some(entry) = xobjects.as_dict().and_then(|d| d.get(name)).cloned() else {
            return;
        };
        let object = self.doc.resolve(&entry);
        let Some(stream) = object.as_stream() else {
            return;
        };
        match stream.dict.name("Subtype") {
            Some("Image") => {
                let m = &state.ctm;
                let corners = [
                    point(m, 0.0, 0.0),
                    point(m, 1.0, 0.0),
                    point(m, 0.0, 1.0),
                    point(m, 1.0, 1.0),
                ];
                let xs = corners.map(|c| c.0);
                let ys = corners.map(|c| c.1);
                let bounds = [
                    xs.iter().copied().fold(f64::MAX, f64::min),
                    ys.iter().copied().fold(f64::MAX, f64::min),
                    xs.iter().copied().fold(f64::MIN, f64::max),
                    ys.iter().copied().fold(f64::MIN, f64::max),
                ];
                self.images.push(Painted {
                    image: Rc::clone(&object),
                    bounds,
                    upright: upright(m),
                });
            }
            Some("Form") => {
                let form_ref = match entry {
                    Object::Ref(r) => Some(r),
                    _ => None,
                };
                if depth >= MAX_FORMS || form_ref.is_some_and(|r| !forms.insert(r)) {
                    return;
                }
                let decoded = self.doc.decoded(stream);
                let form_resources = self.doc.entry(&stream.dict, "Resources");
                let form_resources = form_resources.as_dict().unwrap_or(resources).clone();
                let mut inner = state.clone();
                if let Some(m) = stream.dict.get("Matrix").and_then(Object::as_array) {
                    let v: Vec<f64> = m.iter().filter_map(Object::as_f64).collect();
                    if let [a, b, c, d, e, f] = v[..] {
                        inner.ctm = concat(&[a, b, c, d, e, f], &state.ctm);
                    }
                }
                if decoded.rest.is_none() {
                    self.content(&decoded.data, &form_resources, inner, forms, depth + 1);
                }
                if let Some(r) = form_ref {
                    forms.remove(&r);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------
    /// Past an inline image's data, to its `EI`.
    fn skip_inline_image(&mut self, lexer: &mut Lexer) {
        while let Some(token) = lexer.next_token() {
            if token == Token::Keyword(b"ID") {
                break;
            }
        }
        let data = lexer.data;
        let mut at = lexer.pos + 1;
        while at + 2 <= data.len() {
            let ends = &data[at..at + 2] == b"EI"
                && is_space(data[at - 1])
                && data.get(at + 2).is_none_or(|&b| is_space(b));
            if ends {
                lexer.pos = at + 2;
                return;
            }
            at += 1;
        }
        lexer.pos = data.len();
    }

    // ------------------------------------------------------------------------
    /// How much of the visible `area` the largest image covers, and that
    /// image as the page's scan if it covers the page and is a JPEG.
    fn scan(&self, area: &[f64; 4], turns: u8) -> (f64, Option<Scan>) {
        let corners = [(area[0], area[1]), (area[2], area[3])];
        let shown = corners.map(|(x, y)| point(&turned(turns), x, y));
        let page = [
            shown[0].0.min(shown[1].0),
            shown[0].1.min(shown[1].1),
            shown[0].0.max(shown[1].0),
            shown[0].1.max(shown[1].1),
        ];
        let page_area = (page[2] - page[0]) * (page[3] - page[1]);
        if page_area <= 0.0 {
            return (0.0, None);
        }
        let covered = |b: &[f64; 4]| {
            let w = (b[2].min(page[2]) - b[0].max(page[0])).max(0.0);
            let h = (b[3].min(page[3]) - b[1].max(page[1])).max(0.0);
            w * h / page_area
        };
        let largest = self
            .images
            .iter()
            .max_by(|a, b| covered(&a.bounds).total_cmp(&covered(&b.bounds)));
        let Some(painted) = largest else {
            return (0.0, None);
        };
        let cover = covered(&painted.bounds);
        match cover >= SCAN_COVER {
            true => (cover, self.jpeg(painted)),
            false => (cover, None),
        }
    }

    // ------------------------------------------------------------------------
    /// The image `painted` as a scan, if it is a JPEG.
    fn jpeg(&self, painted: &Painted) -> Option<Scan> {
        let stream = painted.image.as_stream()?;
        let decoded = self.doc.decoded(stream);
        if decoded.rest.as_deref() != Some("DCTDecode") {
            return None;
        }
        let space = self.doc.entry(&stream.dict, "ColorSpace");
        let cmyk = match &*space {
            Object::Name(name) => name == "DeviceCMYK",
            Object::Array(items) => {
                let n = items.get(1).map(|o| self.doc.resolve(o));
                let n = n
                    .as_deref()
                    .and_then(Object::as_dict)
                    .and_then(|d| d.get("N").and_then(Object::as_int));
                items.first().and_then(Object::as_name) == Some("ICCBased") && n == Some(4)
            }
            _ => false,
        };
        Some(Scan {
            jpeg: decoded.data,
            upright: painted.upright,
            cmyk,
        })
    }
}

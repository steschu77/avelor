//! A PDF file's objects and pages: its cross-reference sections (tables and
//! streams, the `/Prev` chain, object streams), rebuilt by scanning the file
//! for `n g obj` when they don't match it, and the page tree with what pages
//! inherit.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::Error;
use crate::content::{self, Page};
use crate::filter::{self, Decoded};
use crate::font::Font;
use crate::lexer::{Lexer, Token, is_space};
use crate::object::{Dict, Object, Ref, Stream};

/// How deep the page tree, and resolving one object for another, go.
const MAX_DEPTH: u32 = 32;
/// More pages than a document read here has.
const MAX_PAGES: usize = 10_000;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// At this offset in the file.
    At(usize),
    /// The `index`th object of the object stream `stream`.
    InStream { stream: u32, index: usize },
}

/// An object stream's objects, by number, in order.
type ObjectStream = Rc<Vec<(u32, Object)>>;

// ----------------------------------------------------------------------------
/// What a page inherits from the page tree, with its own dictionary.
#[derive(Clone, Debug)]
pub(crate) struct PageNode {
    pub dict: Dict,
    pub resources: Object,
    /// The visible area: the crop box, else the media box.
    pub area: [f64; 4],
    /// Clockwise quarter turns it is shown at.
    pub turns: u8,
}

// ----------------------------------------------------------------------------
pub struct Document<'a> {
    data: &'a [u8],
    entries: HashMap<u32, Entry>,
    trailer: Dict,
    pages: Vec<PageNode>,
    objects: RefCell<HashMap<u32, Rc<Object>>>,
    object_streams: RefCell<HashMap<u32, ObjectStream>>,
    pub(crate) fonts: RefCell<HashMap<Ref, Rc<Font>>>,
    depth: Cell<u32>,
}

impl<'a> Document<'a> {
    // ------------------------------------------------------------------------
    /// Read the file `data`: its objects' places and its pages. An encrypted
    /// file is refused: its strings and streams can't be read.
    pub fn parse(data: &'a [u8]) -> Result<Document<'a>, Error> {
        if find(data, b"%PDF-", 0).is_none() {
            return Err(Error::NotPdf);
        }
        let mut doc = Document {
            data,
            entries: HashMap::new(),
            trailer: Dict::default(),
            pages: Vec::new(),
            objects: RefCell::default(),
            object_streams: RefCell::default(),
            fonts: RefCell::default(),
            depth: Cell::new(0),
        };
        let mut rebuilt = false;
        if doc.read_sections().is_none() || !doc.offsets_match() {
            doc.rebuild();
            rebuilt = true;
        }
        if doc.trailer.get("Encrypt").is_some() {
            return Err(Error::Encrypted);
        }
        doc.pages = doc.page_tree();
        if doc.pages.is_empty() && !rebuilt {
            doc.rebuild();
            if doc.trailer.get("Encrypt").is_some() {
                return Err(Error::Encrypted);
            }
            doc.pages = doc.page_tree();
        }
        if doc.pages.is_empty() {
            return Err(Error::Broken("no pages"));
        }
        Ok(doc)
    }

    // ------------------------------------------------------------------------
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    // ------------------------------------------------------------------------
    /// The page `index` (from 0) read: its text, whether it is blank, its
    /// scan. What can't be read on it is left out.
    pub fn page(&self, index: usize) -> Option<Page> {
        let node = self.pages.get(index)?;
        Some(content::read_page(self, node))
    }

    // ------------------------------------------------------------------------
    /// The object numbered `num`, read once.
    pub(crate) fn get(&self, num: u32) -> Option<Rc<Object>> {
        if let Some(object) = self.objects.borrow().get(&num) {
            return Some(Rc::clone(object));
        }
        let entry = *self.entries.get(&num)?;
        let depth = self.depth.get();
        if depth >= MAX_DEPTH {
            return None;
        }
        self.depth.set(depth + 1);
        let object = match entry {
            Entry::At(offset) => self
                .indirect_at(offset)
                .filter(|(r, _)| r.num == num)
                .map(|(_, object)| object),
            Entry::InStream { stream, index } => self.in_stream(stream, index, num),
        };
        self.depth.set(depth);
        let object = Rc::new(object?);
        self.objects.borrow_mut().insert(num, Rc::clone(&object));
        Some(object)
    }

    // ------------------------------------------------------------------------
    /// `object` itself, or what it refers to (`Null` for a reference to
    /// nothing).
    pub(crate) fn resolve(&self, object: &Object) -> Rc<Object> {
        match object {
            Object::Ref(r) => self.get(r.num).unwrap_or_else(|| Rc::new(Object::Null)),
            other => Rc::new(other.clone()),
        }
    }

    // ------------------------------------------------------------------------
    /// The entry `key` of `dict`, resolved.
    pub(crate) fn entry(&self, dict: &Dict, key: &str) -> Rc<Object> {
        match dict.get(key) {
            Some(object) => self.resolve(object),
            None => Rc::new(Object::Null),
        }
    }

    // ------------------------------------------------------------------------
    /// A stream's bytes with its filters undone as far as they can be.
    pub(crate) fn decoded(&self, stream: &Stream) -> Decoded {
        let filters = self.entry(&stream.dict, "Filter");
        let params = self.entry(&stream.dict, "DecodeParms");
        let names: Vec<String> = match &*filters {
            Object::Name(name) => vec![name.clone()],
            Object::Array(items) => items
                .iter()
                .filter_map(|o| self.resolve(o).as_name().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        let params: Vec<Option<Dict>> = match &*params {
            Object::Dict(dict) => vec![Some(dict.clone())],
            Object::Array(items) => items
                .iter()
                .map(|o| self.resolve(o).as_dict().cloned())
                .collect(),
            _ => Vec::new(),
        };
        let filters: Vec<(String, Option<Dict>)> = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, params.get(i).cloned().flatten()))
            .collect();
        filter::decode(&filters, &stream.data)
    }

    // ------------------------------------------------------------------------
    /// The sections from the last `startxref` back through `/Prev`; the
    /// newest entry for an object wins. `None` if the first can't be read.
    fn read_sections(&mut self) -> Option<()> {
        let at = rfind(self.data, b"startxref")?;
        let mut lexer = Lexer::new(self.data, at + b"startxref".len());
        let Some(Token::Int(offset)) = lexer.next_token() else {
            return None;
        };
        let mut offset = usize::try_from(offset).ok()?;
        let mut seen = HashSet::new();
        let mut first = true;
        while seen.insert(offset) {
            let Some(trailer) = self.read_section(offset) else {
                if first {
                    return None;
                }
                break;
            };
            if let Some(stream) = trailer.get("XRefStm").and_then(Object::as_int) {
                if let Ok(stream) = usize::try_from(stream) {
                    if seen.insert(stream) {
                        self.read_section(stream);
                    }
                }
            }
            let prev = trailer.get("Prev").and_then(Object::as_int);
            if first {
                self.trailer = trailer;
                first = false;
            }
            match prev.and_then(|p| usize::try_from(p).ok()) {
                Some(prev) => offset = prev,
                None => break,
            }
        }
        Some(())
    }

    // ------------------------------------------------------------------------
    /// The table or stream at `offset`: its entries added where none is yet,
    /// its trailer (a stream's dictionary).
    fn read_section(&mut self, offset: usize) -> Option<Dict> {
        let mut lexer = Lexer::new(self.data, offset);
        if lexer.peek_keyword(b"xref") {
            lexer.next_token();
            return self.read_table(&mut lexer);
        }
        let (_, object) = self.indirect_at(offset)?;
        let stream = object.as_stream()?;
        let dict = &stream.dict;
        let int = |o: &Object| o.as_int().and_then(|i| usize::try_from(i).ok());
        let widths: Vec<usize> = dict
            .get("W")?
            .as_array()?
            .iter()
            .map(|o| int(o).unwrap_or(0).min(8))
            .collect();
        let [w1, w2, w3] = widths[..] else {
            return None;
        };
        let size = dict.get("Size").and_then(int).unwrap_or(0);
        let index: Vec<usize> = match dict.get("Index").and_then(Object::as_array) {
            Some(items) => items.iter().filter_map(int).collect(),
            None => vec![0, size],
        };
        let data = self.decoded(stream).data;
        let row = w1 + w2 + w3;
        if row == 0 {
            return None;
        }
        let field = |bytes: &[u8]| bytes.iter().fold(0usize, |v, &b| v << 8 | b as usize);
        let mut rows = data.chunks_exact(row);
        for [first, count] in index.as_chunks().0 {
            for num in *first..first.saturating_add(*count) {
                let Some(row) = rows.next() else {
                    break;
                };
                let kind = if w1 == 0 { 1 } else { field(&row[..w1]) };
                let (a, b) = (field(&row[w1..w1 + w2]), field(&row[w1 + w2..]));
                let entry = match kind {
                    1 => Entry::At(a),
                    2 => Entry::InStream {
                        stream: a as u32,
                        index: b,
                    },
                    _ => continue,
                };
                self.entries.entry(num as u32).or_insert(entry);
            }
        }
        Some(dict.clone())
    }

    // ------------------------------------------------------------------------
    /// A table's subsections after `xref`, then its trailer.
    fn read_table(&mut self, lexer: &mut Lexer) -> Option<Dict> {
        loop {
            match lexer.next_token()? {
                Token::Int(start) => {
                    let Some(Token::Int(count)) = lexer.next_token() else {
                        return None;
                    };
                    for i in 0..count.clamp(0, 10_000_000) {
                        let (
                            Some(Token::Int(offset)),
                            Some(Token::Int(_)),
                            Some(Token::Keyword(kind)),
                        ) = (lexer.next_token(), lexer.next_token(), lexer.next_token())
                        else {
                            return None;
                        };
                        if kind == b"n" && offset > 0 {
                            let num = (start + i) as u32;
                            self.entries
                                .entry(num)
                                .or_insert(Entry::At(offset as usize));
                        }
                    }
                }
                Token::Keyword(b"trailer") => return lexer.object()?.as_dict().cloned(),
                _ => return None,
            }
        }
    }

    // ------------------------------------------------------------------------
    /// Whether every object's offset has its `n g obj` there.
    fn offsets_match(&self) -> bool {
        !self.entries.is_empty()
            && self.entries.iter().all(|(&num, entry)| match entry {
                Entry::At(offset) => header_at(self.data, *offset).is_some_and(|(n, _)| n == num),
                Entry::InStream { .. } => true,
            })
    }

    // ------------------------------------------------------------------------
    /// Find every object by scanning the file for `n g obj` (the last one of
    /// a number wins, as an update appends), and the objects in object
    /// streams; the trailer is the last `trailer`, else what an xref stream
    /// or the catalog gives.
    fn rebuild(&mut self) {
        self.entries.clear();
        self.objects.borrow_mut().clear();
        self.object_streams.borrow_mut().clear();
        let mut from = 0;
        while let Some(at) = find(self.data, b"obj", from) {
            from = at + 3;
            if self
                .data
                .get(at + 3)
                .is_some_and(|&b| b.is_ascii_alphanumeric())
            {
                continue;
            }
            if let Some((num, start)) = header_before(self.data, at) {
                self.entries.insert(num, Entry::At(start));
            }
        }
        let mut nums: Vec<u32> = self.entries.keys().copied().collect();
        nums.sort_unstable();
        for &num in &nums {
            let Some(object) = self.get(num) else {
                continue;
            };
            let Some(dict) = object.as_dict() else {
                continue;
            };
            if dict.name("Type") == Some("ObjStm") {
                if let Some(list) = self.object_stream(num) {
                    for (index, (inner, _)) in list.iter().enumerate() {
                        self.entries
                            .entry(*inner)
                            .or_insert(Entry::InStream { stream: num, index });
                    }
                }
            }
        }

        self.trailer = rfind(self.data, b"trailer")
            .and_then(|at| Lexer::new(self.data, at + b"trailer".len()).object())
            .and_then(|o| o.as_dict().cloned())
            .unwrap_or_default();
        for &num in &nums {
            if self.trailer.get("Root").is_some() {
                break;
            }
            let Some(object) = self.get(num) else {
                continue;
            };
            let Some(dict) = object.as_dict() else {
                continue;
            };
            match dict.name("Type") {
                Some("XRef") => {
                    for key in ["Root", "Encrypt"] {
                        if let Some(value) = dict.get(key) {
                            self.trailer.set(key, value.clone());
                        }
                    }
                }
                Some("Catalog") => self.trailer.set("Root", Object::Ref(Ref::new(num))),
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------------
    /// The object at `offset`, `n g obj` first; a stream's bytes found by its
    /// length, or by its `endstream` when the length is wrong.
    fn indirect_at(&self, offset: usize) -> Option<(Ref, Object)> {
        let (num, generation) = header_at(self.data, offset)?;
        let mut lexer = Lexer::new(self.data, offset);
        for _ in 0..3 {
            lexer.next_token();
        }
        let object = lexer.object().unwrap_or(Object::Null);
        let r = Ref { num, generation };
        let Object::Dict(dict) = object else {
            return Some((r, object));
        };
        if !lexer.peek_keyword(b"stream") {
            return Some((r, Object::Dict(dict)));
        }
        lexer.skip_space();
        let mut start = lexer.pos + b"stream".len();
        if self.data.get(start) == Some(&b'\r') {
            start += 1;
        }
        if self.data.get(start) == Some(&b'\n') {
            start += 1;
        }
        let length = match dict.get("Length") {
            Some(Object::Ref(length)) => self.get(length.num).and_then(|o| o.as_int()),
            Some(length) => length.as_int(),
            None => None,
        };
        let by_length = length
            .and_then(|l| usize::try_from(l).ok())
            .and_then(|l| start.checked_add(l))
            .filter(|&end| {
                let mut after = Lexer::new(self.data, end.min(self.data.len()));
                end <= self.data.len() && after.peek_keyword(b"endstream")
            });
        let end = by_length.or_else(|| {
            let found = find(self.data, b"endstream", start)?;
            let mut end = found;
            if end > start && self.data[end - 1] == b'\n' {
                end -= 1;
            }
            if end > start && self.data[end - 1] == b'\r' {
                end -= 1;
            }
            Some(end)
        })?;
        let data = self.data[start..end].to_vec();
        Some((r, Object::Stream(Stream { dict, data })))
    }

    // ------------------------------------------------------------------------
    /// The object `num`, the `index`th of the object stream `stream`.
    fn in_stream(&self, stream: u32, index: usize, num: u32) -> Option<Object> {
        let list = self.object_stream(stream)?;
        let by_index = list.get(index).filter(|(n, _)| *n == num);
        let (_, object) = by_index.or_else(|| list.iter().find(|(n, _)| *n == num))?;
        Some(object.clone())
    }

    // ------------------------------------------------------------------------
    /// The objects of the object stream `num`, read once.
    fn object_stream(&self, num: u32) -> Option<ObjectStream> {
        if let Some(list) = self.object_streams.borrow().get(&num) {
            return Some(Rc::clone(list));
        }
        let object = self.get(num)?;
        let stream = object.as_stream()?;
        let int = |key: &str| stream.dict.get(key).and_then(Object::as_int);
        let (count, first) = (int("N")?.clamp(0, 1_000_000), int("First")?);
        let data = self.decoded(stream).data;
        let mut header = Lexer::new(&data, 0);
        let mut list = Vec::new();
        for _ in 0..count {
            let (Some(Token::Int(inner)), Some(Token::Int(offset))) =
                (header.next_token(), header.next_token())
            else {
                break;
            };
            let at = usize::try_from(first + offset).unwrap_or(usize::MAX);
            if at >= data.len() {
                continue;
            }
            let object = Lexer::new(&data, at).object().unwrap_or(Object::Null);
            list.push((inner as u32, object));
        }
        let list = Rc::new(list);
        self.object_streams
            .borrow_mut()
            .insert(num, Rc::clone(&list));
        Some(list)
    }

    // ------------------------------------------------------------------------
    /// The pages in order, from the catalog's `/Pages`; a node seen before
    /// (a cycle) is skipped.
    fn page_tree(&self) -> Vec<PageNode> {
        let mut pages = Vec::new();
        let root = self.entry(&self.trailer, "Root");
        let Some(root) = root.as_dict() else {
            return pages;
        };
        let Some(tree) = root.get("Pages") else {
            return pages;
        };
        let inherited = PageNode {
            dict: Dict::default(),
            resources: Object::Null,
            area: [0.0, 0.0, 612.0, 792.0],
            turns: 0,
        };
        let mut seen = HashSet::new();
        self.walk(tree, &inherited, &mut seen, &mut pages, 0);
        pages
    }

    // ------------------------------------------------------------------------
    fn walk(
        &self,
        node: &Object,
        inherited: &PageNode,
        seen: &mut HashSet<u32>,
        pages: &mut Vec<PageNode>,
        depth: u32,
    ) {
        if depth > MAX_DEPTH || pages.len() >= MAX_PAGES {
            return;
        }
        if let Object::Ref(r) = node {
            if !seen.insert(r.num) {
                return;
            }
        }
        let object = self.resolve(node);
        let Some(dict) = object.as_dict() else {
            return;
        };
        let mut here = inherited.clone();
        if let Some(resources) = dict.get("Resources") {
            here.resources = resources.clone();
        }
        let boxes = [dict.get("MediaBox"), dict.get("CropBox")];
        for area in boxes.into_iter().flatten() {
            if let Some(area) = self.rectangle(area) {
                here.area = area;
            }
        }
        if let Some(rotate) = self.entry(dict, "Rotate").as_int() {
            here.turns = (rotate.rem_euclid(360) / 90) as u8;
        }
        let kids = self.entry(dict, "Kids");
        match kids.as_array() {
            Some(kids) if dict.name("Type") != Some("Page") => {
                for kid in kids {
                    self.walk(kid, &here, seen, pages, depth + 1);
                }
            }
            _ => {
                here.dict = dict.clone();
                pages.push(here);
            }
        }
    }

    // ------------------------------------------------------------------------
    /// A rectangle `[x1 y1 x2 y2]`, its corners in order.
    pub(crate) fn rectangle(&self, object: &Object) -> Option<[f64; 4]> {
        let object = self.resolve(object);
        let items = object.as_array()?;
        let mut v = [0.0; 4];
        for (slot, item) in v.iter_mut().zip(items) {
            *slot = self.resolve(item).as_f64()?;
        }
        if items.len() < 4 {
            return None;
        }
        Some([
            v[0].min(v[2]),
            v[1].min(v[3]),
            v[0].max(v[2]),
            v[1].max(v[3]),
        ])
    }
}

// ----------------------------------------------------------------------------
pub(crate) fn find(data: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let hay = data.get(from..)?;
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

// ----------------------------------------------------------------------------
fn rfind(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len()).rposition(|w| w == needle)
}

// ----------------------------------------------------------------------------
/// The `n g` of `n g obj` at `offset`, whitespace before it allowed.
fn header_at(data: &[u8], offset: usize) -> Option<(u32, u16)> {
    let mut lexer = Lexer::new(data, offset);
    match (lexer.next_token(), lexer.next_token(), lexer.next_token()) {
        (Some(Token::Int(num)), Some(Token::Int(generation)), Some(Token::Keyword(b"obj"))) => {
            Some((u32::try_from(num).ok()?, u16::try_from(generation).ok()?))
        }
        _ => None,
    }
}

// ----------------------------------------------------------------------------
/// The object number and start of the `n g obj` whose `obj` is at `at`.
fn header_before(data: &[u8], at: usize) -> Option<(u32, usize)> {
    let digits_before = |mut end: usize| {
        while end > 0 && is_space(data[end - 1]) {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && data[start - 1].is_ascii_digit() {
            start -= 1;
        }
        (start < end).then_some((start, end))
    };
    let (generation, _) = digits_before(at)?;
    let (start, end) = digits_before(generation)?;
    if start > 0 && !is_space(data[start - 1]) {
        return None;
    }
    let num = std::str::from_utf8(&data[start..end]).ok()?.parse().ok()?;
    Some((num, start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::Writer;

    // ------------------------------------------------------------------------
    /// A document of one empty page, through the writer.
    fn one_page() -> Vec<u8> {
        let mut w = Writer::new();
        let pages = w.reserve();
        let page = w.add(
            Dict::default()
                .with("Type", Object::name("Page"))
                .with("Parent", pages.into())
                .into(),
        );
        w.put(
            pages,
            Dict::default()
                .with("Type", Object::name("Pages"))
                .with("Kids", Object::Array(vec![page.into()]))
                .with("Count", Object::Int(1))
                .with(
                    "MediaBox",
                    Object::Array([0, 0, 595, 842].map(Object::Int).to_vec()),
                )
                .with("Rotate", Object::Int(-90))
                .into(),
        );
        let catalog = w.add(
            Dict::default()
                .with("Type", Object::name("Catalog"))
                .with("Pages", pages.into())
                .into(),
        );
        w.finish(catalog)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn pages_inherit_their_boxes_and_turns() {
        let data = one_page();
        let doc = Document::parse(&data).unwrap();
        assert_eq!(doc.page_count(), 1);
        assert_eq!(doc.pages[0].area, [0.0, 0.0, 595.0, 842.0]);
        assert_eq!(doc.pages[0].turns, 3);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn broken_offsets_and_junk_rebuild_the_table() {
        let mut data = b"junk\r\n".to_vec();
        data.extend(one_page());
        let doc = Document::parse(&data).unwrap();
        assert_eq!(doc.page_count(), 1);

        let mut data = one_page();
        let at = rfind(&data, b"startxref").unwrap();
        data.truncate(at);
        assert_eq!(Document::parse(&data).unwrap().page_count(), 1);

        assert!(matches!(Document::parse(b"hello"), Err(Error::NotPdf)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_encrypted_file_is_refused() {
        let data = one_page();
        let at = rfind(&data, b"trailer\n<<").unwrap() + b"trailer\n<<".len();
        let mut encrypted = data[..at].to_vec();
        encrypted.extend(b"/Encrypt 9 0 R ");
        encrypted.extend(&data[at..]);
        assert!(matches!(Document::parse(&encrypted), Err(Error::Encrypted)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn xref_streams_and_object_streams_are_read() {
        // Objects 1 (catalog) and 2 (pages) in object stream 3, object 4 the
        // page, the xref stream 5 with PNG Up rows.
        let mut data = b"%PDF-1.5\n".to_vec();
        let page_at = data.len();
        data.extend(b"4 0 obj\n<</Type/Page/Parent 2 0 R>>\nendobj\n");
        let inner = b"1 0 2 29 <</Type/Catalog/Pages 2 0 R>> <</Type/Pages/Kids[4 0 R]/Count 1>>";
        let objstm_at = data.len();
        data.extend(
            format!(
                "3 0 obj\n<</Type/ObjStm/N 2/First 9/Length {}>>\nstream\n",
                inner.len()
            )
            .as_bytes(),
        );
        data.extend(inner);
        data.extend(b"\nendstream\nendobj\n");
        let xref_at = data.len();
        let rows: [[u8; 4]; 6] = [
            [0, 0, 0, 0],
            [2, 0, 3, 0],
            [2, 0, 3, 1],
            [1, 0, objstm_at as u8, 0],
            [1, 0, page_at as u8, 0],
            [1, 0, xref_at as u8, 0],
        ];
        // Up: each row's bytes less the row before.
        let mut encoded = Vec::new();
        let mut prior = [0u8; 4];
        for row in rows {
            encoded.push(2);
            for i in 0..4 {
                encoded.push(row[i].wrapping_sub(prior[i]));
            }
            prior = row;
        }
        let stream = crate::write::zlib_stored(&encoded);
        data.extend(
            format!(
                "5 0 obj\n<</Type/XRef/Size 6/W[1 2 1]/Root 1 0 R/Filter/FlateDecode\
                 /DecodeParms<</Predictor 12/Columns 4>>/Length {}>>\nstream\n",
                stream.len()
            )
            .as_bytes(),
        );
        data.extend(&stream);
        data.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        assert!(xref_at < 256, "offsets fit a byte");

        let doc = Document::parse(&data).unwrap();
        assert!(matches!(doc.entries.get(&2), Some(Entry::InStream { .. })));
        assert_eq!(doc.page_count(), 1);
    }
}

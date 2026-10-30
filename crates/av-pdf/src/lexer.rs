//! PDF's tokens and its objects as written, for the file's body and for
//! content streams alike. Tolerant: what can't be read is skipped or ends
//! the object, never a panic.

use crate::object::{Dict, Object, Ref};

/// How deep arrays and dictionaries nest before the rest is skipped.
const MAX_DEPTH: usize = 64;

// ----------------------------------------------------------------------------
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    /// `obj`, `R`, `stream`, `true`, a content stream's operator, ….
    Keyword(&'a [u8]),
}

// ----------------------------------------------------------------------------
pub fn is_space(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

// ----------------------------------------------------------------------------
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

// ----------------------------------------------------------------------------
fn is_regular(b: u8) -> bool {
    !is_space(b) && !is_delimiter(b)
}

// ----------------------------------------------------------------------------
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    // ------------------------------------------------------------------------
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Lexer { data, pos }
    }

    // ------------------------------------------------------------------------
    /// Past whitespace and comments.
    pub fn skip_space(&mut self) {
        while let Some(&b) = self.data.get(self.pos) {
            if is_space(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(&b) = self.data.get(self.pos) {
                    if b == b'\r' || b == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    // ------------------------------------------------------------------------
    pub fn next_token(&mut self) -> Option<Token<'a>> {
        self.skip_space();
        let b = *self.data.get(self.pos)?;
        match b {
            b'(' => {
                self.pos += 1;
                Some(Token::Str(self.literal()))
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(Token::DictOpen)
            }
            b'<' => {
                self.pos += 1;
                Some(Token::Str(self.hex()))
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Some(Token::DictClose)
            }
            b'[' => {
                self.pos += 1;
                Some(Token::ArrayOpen)
            }
            b']' => {
                self.pos += 1;
                Some(Token::ArrayClose)
            }
            b'/' => {
                self.pos += 1;
                Some(Token::Name(self.name()))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => Some(self.number()),
            _ if is_regular(b) => {
                let start = self.pos;
                while self.data.get(self.pos).is_some_and(|&b| is_regular(b)) {
                    self.pos += 1;
                }
                Some(Token::Keyword(&self.data[start..self.pos]))
            }
            // A stray `)`, `>`, `{` or `}`: a keyword of its own.
            _ => {
                self.pos += 1;
                Some(Token::Keyword(&self.data[self.pos - 1..self.pos]))
            }
        }
    }

    // ------------------------------------------------------------------------
    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else {
                        break;
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut value = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        value = value * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        // A line continuation.
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    if self.data.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    // ------------------------------------------------------------------------
    fn hex(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(v) = hex_value(b) else {
                continue;
            };
            match high.take() {
                Some(h) => out.push(h << 4 | v),
                None => high = Some(v),
            }
        }
        if let Some(h) = high {
            out.push(h << 4);
        }
        out
    }

    // ------------------------------------------------------------------------
    fn name(&mut self) -> String {
        let mut bytes = Vec::new();
        while let Some(&b) = self.data.get(self.pos) {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            let escaped = match (b, self.data.get(self.pos..self.pos + 2)) {
                (b'#', Some(&[h, l])) => hex_value(h).zip(hex_value(l)),
                _ => None,
            };
            match escaped {
                Some((h, l)) => {
                    bytes.push(h << 4 | l);
                    self.pos += 2;
                }
                None => bytes.push(b),
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    // ------------------------------------------------------------------------
    /// A number; signs and points out of place (`--5`, `1.2.3`) read as far
    /// as they make sense.
    fn number(&mut self) -> Token<'a> {
        let start = self.pos;
        while self
            .data
            .get(self.pos)
            .is_some_and(|&b| matches!(b, b'+' | b'-' | b'.' | b'0'..=b'9'))
        {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.data[start..self.pos]);
        let negative = text.starts_with('-');
        let body = text.trim_start_matches(['+', '-']);
        let body: String = {
            let mut seen_point = false;
            body.chars()
                .take_while(|&c| match c {
                    '.' if !seen_point => {
                        seen_point = true;
                        true
                    }
                    '0'..='9' => true,
                    _ => false,
                })
                .collect()
        };
        let sign = if negative { -1.0 } else { 1.0 };
        if body.contains('.') {
            let value = body.parse::<f64>().unwrap_or(0.0);
            Token::Real(sign * value)
        } else {
            let value = body.parse::<i64>().unwrap_or(0);
            Token::Int(if negative { -value } else { value })
        }
    }

    // ------------------------------------------------------------------------
    /// The next object: a reference `n g R` where one is written, arrays and
    /// dictionaries whole. `None` at the end, or at a token that starts no
    /// object (a keyword other than `true`, `false` or `null`, a closing
    /// bracket), which is left unread.
    pub fn object(&mut self) -> Option<Object> {
        self.object_at(0)
    }

    // ------------------------------------------------------------------------
    fn object_at(&mut self, depth: usize) -> Option<Object> {
        let start = self.pos;
        let token = self.next_token()?;
        Some(match token {
            Token::Int(num) => {
                let after = self.pos;
                if let (Some(Token::Int(generation)), Some(Token::Keyword(b"R"))) =
                    (self.next_token(), self.next_token())
                {
                    if let (Ok(num), Ok(generation)) =
                        (u32::try_from(num), u16::try_from(generation))
                    {
                        return Some(Object::Ref(Ref { num, generation }));
                    }
                }
                self.pos = after;
                Object::Int(num)
            }
            Token::Real(r) => Object::Real(r),
            Token::Name(name) => Object::Name(name),
            Token::Str(bytes) => Object::Str(bytes),
            Token::ArrayOpen => {
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.data.get(self.pos) {
                        None => break,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        _ => {}
                    }
                    if depth >= MAX_DEPTH {
                        self.pos += 1;
                        continue;
                    }
                    match self.object_at(depth + 1) {
                        Some(item) => items.push(item),
                        // A keyword inside an array: skipped.
                        None => {
                            if self.next_token().is_none() {
                                break;
                            }
                        }
                    }
                }
                Object::Array(items)
            }
            Token::DictOpen => Object::Dict(self.dict_body(depth)),
            Token::Keyword(b"true") => Object::Bool(true),
            Token::Keyword(b"false") => Object::Bool(false),
            Token::Keyword(b"null") => Object::Null,
            Token::Keyword(_) | Token::ArrayClose | Token::DictClose => {
                self.pos = start;
                return None;
            }
        })
    }

    // ------------------------------------------------------------------------
    /// A dictionary's entries after its `<<`, through its `>>`.
    fn dict_body(&mut self, depth: usize) -> Dict {
        let mut dict = Dict::default();
        loop {
            let before = self.pos;
            match self.next_token() {
                None | Some(Token::DictClose) => break,
                Some(Token::Name(key)) => {
                    if depth >= MAX_DEPTH {
                        continue;
                    }
                    // A key without a value (`/K >>`, `/K endobj`) leaves
                    // the next token to this loop.
                    if let Some(value) = self.object_at(depth + 1) {
                        dict.0.push((key, value));
                    }
                }
                // A dictionary that ends without its `>>`.
                Some(Token::Keyword(b"endobj" | b"stream")) => {
                    self.pos = before;
                    break;
                }
                Some(_) => {}
            }
        }
        dict
    }

    // ------------------------------------------------------------------------
    /// Whether the next token is the keyword `word`, which stays unread.
    pub fn peek_keyword(&mut self, word: &[u8]) -> bool {
        let pos = self.pos;
        let found = matches!(self.next_token(), Some(Token::Keyword(k)) if k == word);
        self.pos = pos;
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn object(text: &[u8]) -> Option<Object> {
        Lexer::new(text, 0).object()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn strings_and_names_resolve_their_escapes() {
        assert_eq!(
            object(b"(a\\(b\\) (c) \\101\\60\\\r\nd\re)"),
            Some(Object::Str(b"a(b) (c) A0d\ne".to_vec()))
        );
        assert_eq!(
            object(b"<48 65 6C6C 6F7>"),
            Some(Object::Str(b"Hellop".to_vec()))
        );
        assert_eq!(object(b"/A#20B#2"), Some(Object::name("A B#2")));
        assert_eq!(object(b"% comment\n -.5"), Some(Object::Real(-0.5)));
        assert_eq!(object(b"--7"), Some(Object::Int(-7)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn references_arrays_and_dictionaries_nest() {
        let dict = object(b"<</Kids [1 0 R 2 3 [true null]] /Count 2 /Empty>>").unwrap();
        let dict = dict.as_dict().unwrap();
        assert_eq!(
            dict.get("Kids"),
            Some(&Object::Array(vec![
                Object::Ref(Ref::new(1)),
                Object::Int(2),
                Object::Int(3),
                Object::Array(vec![Object::Bool(true), Object::Null]),
            ]))
        );
        assert_eq!(dict.get("Count"), Some(&Object::Int(2)));

        let mut lexer = Lexer::new(b"<</Length 5>>stream\nhello", 0);
        assert!(lexer.object().unwrap().as_dict().is_some());
        assert!(lexer.peek_keyword(b"stream"));
        let mut lexer = Lexer::new(b"<</A 1 endobj", 0);
        assert_eq!(lexer.object().unwrap().as_dict().unwrap().0.len(), 1);
        assert!(lexer.peek_keyword(b"endobj"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn deep_nesting_stops_without_overflow() {
        let mut text = vec![b'['; 10_000];
        text.extend(vec![b']'; 10_000]);
        assert!(object(&text).is_some());
    }
}

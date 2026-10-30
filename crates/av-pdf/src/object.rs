//! PDF objects, and writing them back as PDF syntax.

use std::fmt::Write;

// ----------------------------------------------------------------------------
/// An indirect reference, `n g R`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ref {
    pub num: u32,
    pub generation: u16,
}

impl Ref {
    // ------------------------------------------------------------------------
    pub fn new(num: u32) -> Ref {
        Ref { num, generation: 0 }
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// Without its slash, `#xx` escapes resolved.
    Name(String),
    /// A literal or hex string's bytes.
    Str(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(Ref),
}

// ----------------------------------------------------------------------------
/// A dictionary's entries in their order; dictionaries are small, so a
/// lookup goes through them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict(pub Vec<(String, Object)>);

// ----------------------------------------------------------------------------
/// A stream's dictionary and its bytes as stored (still encoded).
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
}

impl Object {
    // ------------------------------------------------------------------------
    pub fn name(name: &str) -> Object {
        Object::Name(name.to_string())
    }

    // ------------------------------------------------------------------------
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Object::Name(name) => Some(name),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_str(&self) -> Option<&[u8]> {
        match self {
            Object::Str(bytes) => Some(bytes),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(items) => Some(items),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    /// A dictionary, or a stream's.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(dict) => Some(dict),
            Object::Stream(stream) => Some(&stream.dict),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(stream) => Some(stream),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    /// The object as PDF syntax.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        match self {
            Object::Null => out.extend_from_slice(b"null"),
            Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Object::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
            Object::Real(r) => {
                let mut text = format!("{r:.4}");
                while text.ends_with('0') {
                    text.pop();
                }
                if text.ends_with('.') {
                    text.pop();
                }
                out.extend_from_slice(text.as_bytes());
            }
            Object::Name(name) => {
                let mut text = String::from("/");
                for b in name.bytes() {
                    match b {
                        b'!'..=b'~' if !b"#()<>[]{}/%".contains(&b) => text.push(b as char),
                        _ => {
                            let _ = write!(text, "#{b:02X}");
                        }
                    }
                }
                out.extend_from_slice(text.as_bytes());
            }
            Object::Str(bytes) => {
                out.push(b'(');
                for &b in bytes {
                    match b {
                        b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', b]),
                        // A reader takes a bare CR as the end of a line.
                        b'\r' => out.extend_from_slice(b"\\r"),
                        _ => out.push(b),
                    }
                }
                out.push(b')');
            }
            Object::Array(items) => {
                out.push(b'[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(b' ');
                    }
                    item.write_to(out);
                }
                out.push(b']');
            }
            Object::Dict(dict) => dict.write_to(out),
            Object::Stream(stream) => {
                let mut dict = stream.dict.clone();
                dict.set("Length", Object::Int(stream.data.len() as i64));
                dict.write_to(out);
                out.extend_from_slice(b"\nstream\n");
                out.extend_from_slice(&stream.data);
                out.extend_from_slice(b"\nendstream");
            }
            Object::Ref(r) => {
                out.extend_from_slice(format!("{} {} R", r.num, r.generation).as_bytes());
            }
        }
    }
}

impl Dict {
    // ------------------------------------------------------------------------
    pub fn get(&self, key: &str) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    // ------------------------------------------------------------------------
    /// Set `key` to `value`, in place of what it held.
    pub fn set(&mut self, key: &str, value: Object) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    // ------------------------------------------------------------------------
    /// With `key` set to `value`.
    pub fn with(mut self, key: &str, value: Object) -> Dict {
        self.set(key, value);
        self
    }

    // ------------------------------------------------------------------------
    pub fn name(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Object::as_name)
    }

    // ------------------------------------------------------------------------
    fn write_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"<<");
        for (key, value) in &self.0 {
            Object::Name(key.clone()).write_to(out);
            out.push(b' ');
            value.write_to(out);
            out.push(b' ');
        }
        out.extend_from_slice(b">>");
    }
}

impl From<Dict> for Object {
    fn from(dict: Dict) -> Object {
        Object::Dict(dict)
    }
}

impl From<Ref> for Object {
    fn from(r: Ref) -> Object {
        Object::Ref(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn written(object: &Object) -> String {
        let mut out = Vec::new();
        object.write_to(&mut out);
        String::from_utf8(out).unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn objects_write_as_pdf_syntax() {
        let dict = Dict::default()
            .with("Type", Object::name("Page"))
            .with("A B", Object::Real(0.5))
            .with(
                "Kids",
                Object::Array(vec![Ref::new(3).into(), Object::Int(-2)]),
            )
            .with("T", Object::Str(b"a(b)\\\r".to_vec()));
        assert_eq!(
            written(&dict.into()),
            "<</Type /Page /A#20B 0.5 /Kids [3 0 R -2] /T (a\\(b\\)\\\\\\r) >>"
        );
        assert_eq!(written(&Object::Real(595.0)), "595");
        let stream = Object::Stream(Stream {
            dict: Dict::default(),
            data: b"BT".to_vec(),
        });
        assert_eq!(written(&stream), "<</Length 2 >>\nstream\nBT\nendstream");
    }
}

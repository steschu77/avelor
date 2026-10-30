//! Writing a PDF file: objects numbered as they are added, then the
//! cross-reference table and trailer.

use crate::object::{Dict, Object, Ref};

// ----------------------------------------------------------------------------
pub struct Writer {
    out: Vec<u8>,
    /// Each object's offset, by number less one; `None` while reserved.
    offsets: Vec<Option<usize>>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    // ------------------------------------------------------------------------
    pub fn new() -> Writer {
        Writer {
            out: b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec(),
            offsets: Vec::new(),
        }
    }

    // ------------------------------------------------------------------------
    /// A number for an object written later with `put`, so others can refer
    /// to it first.
    pub fn reserve(&mut self) -> Ref {
        self.offsets.push(None);
        Ref::new(self.offsets.len() as u32)
    }

    // ------------------------------------------------------------------------
    /// Write `object` as the reserved `r`.
    pub fn put(&mut self, r: Ref, object: Object) {
        if let Some(slot) = self.offsets.get_mut(r.num as usize - 1) {
            *slot = Some(self.out.len());
        }
        self.out
            .extend_from_slice(format!("{} 0 obj\n", r.num).as_bytes());
        object.write_to(&mut self.out);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    // ------------------------------------------------------------------------
    pub fn add(&mut self, object: Object) -> Ref {
        let r = self.reserve();
        self.put(r, object);
        r
    }

    // ------------------------------------------------------------------------
    /// The file, with `root` as its catalog.
    pub fn finish(mut self, root: Ref) -> Vec<u8> {
        let xref = self.out.len();
        let size = self.offsets.len() + 1;
        let mut table = format!("xref\n0 {size}\n0000000000 65535 f \n");
        for offset in &self.offsets {
            match offset {
                Some(offset) => table.push_str(&format!("{offset:010} 00000 n \n")),
                None => table.push_str("0000000000 65535 f \n"),
            }
        }
        self.out.extend_from_slice(table.as_bytes());
        self.out.extend_from_slice(b"trailer\n");
        let trailer = Dict::default()
            .with("Size", Object::Int(size as i64))
            .with("Root", root.into());
        Object::Dict(trailer).write_to(&mut self.out);
        self.out
            .extend_from_slice(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        self.out
    }
}

// ----------------------------------------------------------------------------
/// `data` as zlib data in stored (uncompressed) blocks: what `FlateDecode`
/// takes, without a compressor.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(0xffff).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        let len = block.len() as u16;
        out.push(u8::from(last));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use av_utils::inflate;

    // ------------------------------------------------------------------------
    #[test]
    fn stored_zlib_inflates_back() {
        for data in [Vec::new(), b"Hello".to_vec(), vec![7u8; 70_000]] {
            let mut out = Vec::new();
            inflate::zlib(&mut out, &zlib_stored(&data), 1 << 20).unwrap();
            assert_eq!(out, data);
        }
        assert_eq!(&zlib_stored(b"a")[8..], &[0x00, 0x62, 0x00, 0x62]);
    }
}

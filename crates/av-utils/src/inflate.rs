//! RFC 1951 decompression, from the author's miniz-rs
//! (github.com/steschu77/miniz-rs, MIT), writing into a growing buffer.
//! * https://datatracker.ietf.org/doc/html/rfc1951
//! * based on the excellent LodePNG implementation by Lode Vandevenne:
//!   https://lodev.org/lodepng/

// ----------------------------------------------------------------------------
#[derive(Debug, PartialEq)]
pub enum Error {
    Underflow,
    /// The output would pass its limit.
    Overflow,
    InvalidBlockType,
    InvalidBlockLength,
    InvalidCodeLength,
    InvalidCopyLength,
    InvalidDistance,
    InvalidLength,
    InvalidSymbol,
    InvalidData,
    InvalidFirstCopyCode,
    UnderSubscribedTree,
    OverSubscribedTree,
}

// ----------------------------------------------------------------------------
fn show_bits(bp: &usize, src: &[u8], count: u8) -> Result<u16, Error> {
    let bytepos = *bp >> 3;
    let shift = *bp & 7;
    let mask = (1 << count) - 1;

    if bytepos + 2 < src.len() {
        let bits32 = ((src[bytepos + 2] as u32) << 16)
            | ((src[bytepos + 1] as u32) << 8)
            | (src[bytepos] as u32);
        Ok((bits32 >> shift) as u16 & mask)
    } else if bytepos + 1 < src.len() {
        let bits16 = ((src[bytepos + 1] as u16) << 8) | (src[bytepos] as u16);
        Ok((bits16 >> shift) & mask)
    } else if bytepos + 1 == src.len() {
        let bits8 = src[bytepos] as u16;
        Ok((bits8 >> shift) & mask)
    } else {
        Err(Error::Underflow)
    }
}

// ----------------------------------------------------------------------------
fn read_bits(src: &[u8], sptr: &mut usize, count: u8) -> Result<u16, Error> {
    let res = show_bits(sptr, src, count)?;
    *sptr += count as usize;
    Ok(res)
}

// ----------------------------------------------------------------------------
fn reverse_bits(x: u16, count: usize) -> u16 {
    let x = ((x & 0x5555) << 1) | ((x >> 1) & 0x5555);
    let x = ((x & 0x3333) << 2) | ((x >> 2) & 0x3333);
    let x = ((x & 0x0f0f) << 4) | ((x >> 4) & 0x0f0f);
    let x = ((x & 0x00ff) << 8) | ((x >> 8) & 0x00ff);
    x >> (16 - count)
}

// ----------------------------------------------------------------------------
const TABLE_BITS: u8 = 9;

// ----------------------------------------------------------------------------
#[derive(Copy, Clone, Debug)]
struct VarLenCode {
    code: u16,
    len: u8,
}

type LookupTable = [VarLenCode; 512 + 512];

// ------------------------------------------------------------------------
#[allow(clippy::comparison_chain)]
fn generate_codes(codes: &mut [u16], lengths: &[u8]) -> Result<bool, Error> {
    const MAX_CODE_LENGTH: usize = 16;

    // count number of instances of each code length
    let mut code_len_count = [0; MAX_CODE_LENGTH];
    for len in lengths.iter() {
        code_len_count[*len as usize] += 1;
    }

    // monitor for over- or under-subscription by tracking available_codes
    let mut available_codes: i32 = 1;
    for count in code_len_count.iter().take(MAX_CODE_LENGTH).skip(1) {
        available_codes = (available_codes << 1) - *count as i32;
    }

    if available_codes != 0 {
        // For a proper Huffman tree, the sum of all code lengths should match the total number of
        // leaves (symbols) in the binary tree (available_codes == 0).
        return if available_codes == 0x8000 || available_codes == 0x4000 {
            // trivial under-subscriptions: only a single symbol, or no symbols at all
            Ok(false)
        } else if available_codes < 0 {
            Err(Error::OverSubscribedTree)
        } else {
            Err(Error::UnderSubscribedTree)
        };
    }

    // calculate next code for each code length
    let mut next_code = [0u16; MAX_CODE_LENGTH];
    for i in 1..MAX_CODE_LENGTH {
        next_code[i] = next_code[i - 1].wrapping_add(code_len_count[i - 1]) << 1;
    }

    for (len, code) in lengths.iter().zip(codes.iter_mut()) {
        if *len != 0 {
            let len = *len as usize;
            // Huffman bits are given in MSB first order but the bit reader reads LSB first
            *code = reverse_bits(next_code[len], len);
            next_code[len] = next_code[len].wrapping_add(1);
        }
    }

    Ok(true)
}

// ------------------------------------------------------------------------
fn fill_table(table: &mut [VarLenCode], num: usize, offset: usize, len: u8, code: u16) {
    for entry in table[offset..].iter_mut().step_by(1 << len).take(num) {
        *entry = VarLenCode { code, len };
    }
}

// ------------------------------------------------------------------------
fn make_lookup_table(lengths: &[u8]) -> Result<LookupTable, Error> {
    const TABLE_SIZE: usize = 1 << TABLE_BITS; // size of the first table
    const TABLE_MASK: u16 = (1 << TABLE_BITS) - 1;
    let mut table = [VarLenCode { code: 0, len: 1 }; 1024];

    let mut codes = vec![0; lengths.len()];
    if !generate_codes(&mut codes, lengths)? {
        // no codes generated for trivial cases
        return Ok(table);
    }

    // compute maxlens: max total bit length of symbols sharing prefix in the first table
    let mut maxlens = [0; TABLE_SIZE];
    for (len, code) in lengths.iter().zip(codes.iter_mut()) {
        if *len <= TABLE_BITS {
            // symbols that fit in first table don't increase secondary table size
            continue;
        }

        // get the FIRSTBITS MSBs, the MSBs of the symbol are encoded first.
        let index = (*code & TABLE_MASK) as usize;
        maxlens[index] = maxlens[index].max(*len);
    }

    // fill in the first table for long symbols: max prefix size and pointer to secondary tables
    let mut pointer = TABLE_SIZE;
    for i in 0..TABLE_SIZE {
        let l = maxlens[i];
        if l <= TABLE_BITS {
            continue;
        }
        table[i].len = l;
        table[i].code = pointer as u16;

        let scondary_table_size = 1 << (l - TABLE_BITS);
        pointer += scondary_table_size;
    }

    // fill in the first table for short symbols, or secondary table for long symbols
    for (i, (len, code)) in lengths.iter().zip(codes.iter_mut()).enumerate() {
        if *len == 0 {
            continue;
        }

        if *len <= TABLE_BITS {
            // short symbol, fully in first table, replicated num times if l < FIRSTBITS
            let num = 1usize << (TABLE_BITS - *len);
            fill_table(&mut table, num, *code as usize, *len, i as u16);
        } else {
            // long symbol, shares prefix with other long symbols in first lookup table, needs second lookup
            // the FIRSTBITS MSBs of the symbol are the first table index
            let index = (*code & TABLE_MASK) as usize;
            let maxlen = table[index].len;

            // amount of entries of this symbol in secondary table
            let num = 1usize << (maxlen - *len);
            let start = table[index].code as usize;
            let code = *code >> TABLE_BITS;
            let len = *len - TABLE_BITS;
            fill_table(&mut table[start..], num, code as usize, len, i as u16);
        }
    }

    Ok(table)
}

// ----------------------------------------------------------------------------
fn read_symbol(src: &[u8], sptr: &mut usize, lookup_table: &LookupTable) -> Result<u16, Error> {
    let idx = show_bits(sptr, src, TABLE_BITS)? as usize;
    let code_0 = &lookup_table[idx];

    if code_0.len <= TABLE_BITS {
        // short symbol, fully in first table
        *sptr += code_0.len as usize;
        Ok(code_0.code)
    } else {
        // long symbol, needs second lookup, code_0.code points to start of second table
        *sptr += TABLE_BITS as usize;
        let count = code_0.len - TABLE_BITS;

        let idx = show_bits(sptr, src, count)? as usize;
        let code_1 = &lookup_table[code_0.code as usize + idx];

        *sptr += code_1.len as usize;
        Ok(code_1.code)
    }
}

// ----------------------------------------------------------------------------
fn generate_fixed_luts() -> Result<(LookupTable, LookupTable), Error> {
    const NUM_DEFLATE_CODE_SYMBOLS: usize = 288;
    let mut len_ll = [8; NUM_DEFLATE_CODE_SYMBOLS];
    len_ll[144..256].fill(9);
    len_ll[256..280].fill(7);
    let lut_ll = make_lookup_table(&len_ll)?;

    const NUM_DISTANCE_SYMBOLS: usize = 32;
    let len_d = [5; NUM_DISTANCE_SYMBOLS];
    let lut_d = make_lookup_table(&len_d)?;

    Ok((lut_ll, lut_d))
}

// ----------------------------------------------------------------------------
fn read_encoded_luts(src: &[u8], sptr: &mut usize) -> Result<(LookupTable, LookupTable), Error> {
    let ll_len = (read_bits(src, sptr, 5)? + 257) as usize;
    let dt_len = (read_bits(src, sptr, 5)? + 1) as usize;
    let cl_len = (read_bits(src, sptr, 4)? + 4) as usize;

    if ll_len > 286 || dt_len > 30 {
        return Err(Error::InvalidCodeLength);
    }

    const NUM_CODE_LENGTH_CODES: usize = 19;
    let mut len_cl = [0; NUM_CODE_LENGTH_CODES];

    const CODE_LEN_PERM: [u8; NUM_CODE_LENGTH_CODES] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    for cl in &CODE_LEN_PERM[..cl_len] {
        len_cl[*cl as usize] = read_bits(src, sptr, 3)? as u8;
    }

    let vlc_cl = make_lookup_table(&len_cl)?;

    let count = ll_len + dt_len;
    const NUM_DEFLATE_CODE_SYMBOLS: usize = 288;
    const NUM_DISTANCE_SYMBOLS: usize = 32;
    let mut bitlen = [0; NUM_DEFLATE_CODE_SYMBOLS + NUM_DISTANCE_SYMBOLS];

    let mut ptr = 0;
    while ptr < count {
        let code = read_symbol(src, sptr, &vlc_cl)?;
        match code {
            0..=15 => {
                bitlen[ptr] = code as u8;
                ptr += 1;
            }
            16 => {
                if ptr == 0 {
                    return Err(Error::InvalidFirstCopyCode);
                }
                let len = 3 + read_bits(src, sptr, 2)? as usize;
                if ptr + len > count {
                    return Err(Error::InvalidCopyLength);
                }
                let value = bitlen[ptr - 1];
                bitlen[ptr..ptr + len].fill(value);
                ptr += len;
            }
            17 | 18 => {
                let len = if code == 17 {
                    3 + read_bits(src, sptr, 3)?
                } else {
                    11 + read_bits(src, sptr, 7)?
                } as usize;
                if ptr + len > count {
                    return Err(Error::InvalidCopyLength);
                }
                bitlen[ptr..ptr + len].fill(0);
                ptr += len;
            }
            _ => {
                return Err(Error::InvalidData);
            }
        }
    }

    if bitlen[256] == 0 {
        // end-marker is mandatory
        return Err(Error::InvalidData);
    }

    let lut_ll = make_lookup_table(&bitlen[0..ll_len])?;
    let lut_d = make_lookup_table(&bitlen[ll_len..ll_len + dt_len])?;

    Ok((lut_ll, lut_d))
}

// ----------------------------------------------------------------------------
#[rustfmt::skip]
const DIST_INFO: [(u8, u16); 30] = [
    ( 0,    1), ( 0,    2), ( 0,    3), ( 0,    4), ( 1,    5), ( 1,    7), ( 2,    9), ( 2,   13),
    ( 3,   17), ( 3,   25), ( 4,   33), ( 4,   49), ( 5,   65), ( 5,   97), ( 6,  129), ( 6,  193),
    ( 7,  257), ( 7,  385), ( 8,  513), ( 8,  769), ( 9, 1025), ( 9, 1537), (10, 2049), (10, 3073),
    (11, 4097), (11, 6145), (12, 8193), (12,12289), (13,16385), (13,24577),
];

// ----------------------------------------------------------------------------
#[rustfmt::skip]
const CODE_INFO: [(u8, u16); 29] = [
    ( 0,    3), ( 0,    4), ( 0,    5), ( 0,    6), ( 0,    7), ( 0,    8), ( 0,   9), ( 0,   10),
    ( 1,   11), ( 1,   13), ( 1,   15), ( 1,   17), ( 2,   19), ( 2,   23), ( 2,  27), ( 2,   31),
    ( 3,   35), ( 3,   43), ( 3,   51), ( 3,   59), ( 4,   67), ( 4,   83), ( 4,  99), ( 4,  115),
    ( 5,  131), ( 5,  163), ( 5,  195), ( 5,  227), ( 0,  258),
];

// ----------------------------------------------------------------------------
fn inflate_huffman_block(
    dst: &mut Vec<u8>,
    limit: usize,
    src: &[u8],
    sptr: &mut usize,
    trees: (LookupTable, LookupTable),
) -> Result<(), Error> {
    loop {
        let code_ll = read_symbol(src, sptr, &trees.0)?;
        match code_ll {
            0..=255 => {
                if dst.len() >= limit {
                    return Err(Error::Overflow);
                }
                dst.push(code_ll as u8);
            }
            256 => {
                return Ok(());
            }
            257..=285 => {
                let idx = (code_ll - 257) as usize;
                let info_ll = CODE_INFO.get(idx).ok_or(Error::InvalidLength)?;
                let length = info_ll.1 as usize + read_bits(src, sptr, info_ll.0)? as usize;

                let code_d = read_symbol(src, sptr, &trees.1)? as usize;
                let info_d = DIST_INFO.get(code_d).ok_or(Error::InvalidDistance)?;
                let distance = info_d.1 as usize + read_bits(src, sptr, info_d.0)? as usize;

                // A distance before the first byte (miniz-rs underflowed
                // there for a distance of 1).
                let start = dst.len();
                if distance > start {
                    return Err(Error::InvalidDistance);
                }
                if length > limit - start {
                    return Err(Error::Overflow);
                }

                // An overlapping copy repeats the last `distance` bytes.
                let mut left = length;
                while left > 0 {
                    let chunk = left.min(distance);
                    let from = dst.len() - distance;
                    dst.extend_from_within(from..from + chunk);
                    left -= chunk;
                }
            }
            _ => {
                return Err(Error::InvalidSymbol);
            }
        }
    }
}

// ----------------------------------------------------------------------------
fn inflate_no_compression(
    dst: &mut Vec<u8>,
    limit: usize,
    src: &[u8],
    sptr: &mut usize,
) -> Result<(), Error> {
    // align on byte boundary
    *sptr = (*sptr + 7) & (!7);

    let bytepos = *sptr >> 3;

    if bytepos + 4 > src.len() {
        return Err(Error::Underflow);
    }

    let len = src[bytepos] as usize + ((src[bytepos + 1] as usize) << 8);
    let nlen = src[bytepos + 2] as usize + ((src[bytepos + 3] as usize) << 8);

    if len + nlen != 65535 {
        // error: NLEN is not one's complement of LEN
        return Err(Error::InvalidBlockLength);
    }

    if bytepos + 4 + len > src.len() {
        // error, bit pointer will jump past memory
        return Err(Error::Underflow);
    }

    // miniz-rs copied into its fixed buffer without checking its room.
    if len > limit - dst.len() {
        return Err(Error::Overflow);
    }
    dst.extend_from_slice(&src[bytepos + 4..bytepos + 4 + len]);
    *sptr += (4 + len) * 8;

    Ok(())
}

// ----------------------------------------------------------------------------
/// Inflate the raw DEFLATE data `src` onto `dst`, at most `limit` bytes. On
/// an error `dst` keeps what was inflated before it.
pub fn inflate(dst: &mut Vec<u8>, src: &[u8], limit: usize) -> Result<(), Error> {
    let mut sptr = 0;
    loop {
        let b_final = read_bits(src, &mut sptr, 1)?;
        let b_type = read_bits(src, &mut sptr, 2)?;

        match b_type {
            0 => {
                inflate_no_compression(dst, limit, src, &mut sptr)?;
            }
            1 => {
                let trees = generate_fixed_luts()?;
                inflate_huffman_block(dst, limit, src, &mut sptr, trees)?;
            }
            2 => {
                let trees = read_encoded_luts(src, &mut sptr)?;
                inflate_huffman_block(dst, limit, src, &mut sptr, trees)?;
            }
            _ => {
                return Err(Error::InvalidBlockType);
            }
        }

        if b_final != 0 {
            break;
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// Inflate zlib data (RFC 1950), as PDF's `FlateDecode` stores it: its
/// header skipped, its checksum ignored. Data without a valid header is
/// taken as raw DEFLATE, which some writers produce.
pub fn zlib(dst: &mut Vec<u8>, src: &[u8], limit: usize) -> Result<(), Error> {
    let header = match src {
        [cmf, flg, ..] => (*cmf as u16) << 8 | *flg as u16,
        _ => return Err(Error::Underflow),
    };
    let valid = header % 31 == 0 && src[0] & 0x0f == 8;
    let skip = match valid {
        true if src[1] & 0x20 != 0 => 6,
        true => 2,
        false => 0,
    };
    inflate(dst, src.get(skip..).unwrap_or_default(), limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: usize = 1 << 20;

    // ------------------------------------------------------------------------
    fn inflated(src: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        inflate(&mut out, src, LIMIT).map(|()| out)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_zlib_cover_vectors_inflate() {
        // edge-case test vectors from
        // https://github.com/madler/zlib/blob/master/test/infcover.c
        let out = inflated(&[0x01, 0x01, 0x00, 0xfe, 0xff, 0x66]).unwrap();
        assert_eq!(out, [0x66], "stored mode");

        assert!(
            inflated(&[0x03, 0x00]).unwrap().is_empty(),
            "fixed LUTs - no data"
        );

        let out = inflated(&[0x2b, 0x1f, 0x05, 0x40, 0x0c, 0x00]).unwrap();
        assert_eq!((out.len(), out[0]), (262, 0x77), "fixed LUTs - window wrap");

        // dynamic LUTs coverage
        // * all code-tree methods used (length, length repeat, 3 and 7 bit zeros range)
        // * 15 bit literal code
        // * 10 bit distance extra with MSB set spread over last 3 bytes
        let inp = [
            0xed, 0xf6, 0x49, 0x82, 0x24, 0x49, 0x12, 0x04, 0x49, 0xd2, 0xf3, 0xe7, 0xd9, 0xc8,
            0xa2, 0xe6, 0x91, 0x75, 0xec, 0x7d, 0x4e, 0x00, 0xaf, 0x80, 0xff, 0xdf, 0x00, 0x00,
            0xe0, 0x5c, 0x0c, 0x03,
        ];
        let out = inflated(&inp).unwrap();
        assert_eq!(out.len(), 2588, "dynamic LUTs coverage");
        assert_eq!((out[0], out[1]), (0x88, 0x00));
        assert_eq!((out[2588 - 3], out[2588 - 2]), (0x88, 0x00));

        let inp = [
            0xed, 0xf6, 0x49, 0x82, 0x24, 0x49, 0x12, 0x04, 0x49, 0xd2, 0xf3, 0xe7, 0xd9, 0xc8,
            0xa2, 0xe6, 0x91, 0x75, 0xec, 0xbd, 0x4f, 0x00, 0xaf, 0x80, 0x00,
        ];
        assert_eq!(inflated(&inp), Err(Error::OverSubscribedTree));

        // length extra / 1 symbol in dist tree
        let inp = [
            0xed, 0xc0, 0x1, 0x1, 0x0, 0x0, 0x0, 0x40, 0xa0, 0xfb, 0x66, 0x1b, 0x42, 0x2c, 0x4f,
        ];
        let out = inflated(&inp).unwrap();
        assert_eq!((out.len(), out[0]), (516, 0x88), "length extra");

        let mut inp = vec![
            0xed, 0xc0, 0x81, 0x0, 0x0, 0x0, 0x0, 0x80, 0xa0, 0xfd, 0xa9, 0x17, 0xa9,
        ];
        inp.extend([0u8; 32]);
        inp.push(0x6);
        assert_eq!(inflated(&inp).unwrap().len(), 33025, "window end");

        let inp = [0x2, 0x8, 0x20, 0x80, 0x0, 0x3, 0x0];
        assert!(
            inflated(&inp).unwrap().is_empty(),
            "inflate_fast TYPE return"
        );

        assert_eq!(inflated(&[0x06]), Err(Error::InvalidBlockType));
        let inp = [0x01, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(inflated(&inp), Err(Error::InvalidBlockLength));
        assert_eq!(inflated(&[0xfc, 0x00, 0x00]), Err(Error::InvalidCodeLength));
        let inp = [0x04, 0x00, 0xfe, 0xff];
        assert_eq!(inflated(&inp), Err(Error::UnderSubscribedTree));
        let inp = [0x04, 0x00, 0x24, 0x49, 0x00];
        assert_eq!(inflated(&inp), Err(Error::InvalidFirstCopyCode));
        let inp = [0x04, 0x00, 0x24, 0xe9, 0xff, 0xff];
        assert_eq!(inflated(&inp), Err(Error::InvalidCopyLength));
    }

    // ------------------------------------------------------------------------
    const THIS_IS_THIS_TEST: [u8; 25] = [
        0xD3, 0xC5, 0x01, 0xB8, 0x80, 0x58, 0x21, 0xC4, 0xC3, 0x33, 0x58, 0x01, 0x88, 0xC0, 0x74,
        0x88, 0x6B, 0x70, 0x88, 0x02, 0x50, 0x02, 0xA7, 0x0E, 0x00,
    ];
    const THIS_IS_THIS_TEXT: &[u8] =
        b"------------------------\n--- THIS IS THIS TEST --\n------------------------\n";

    // ------------------------------------------------------------------------
    #[test]
    fn a_stream_inflates_with_or_without_its_zlib_header() {
        assert_eq!(inflated(&THIS_IS_THIS_TEST).unwrap(), THIS_IS_THIS_TEXT);

        let mut zlib_data = vec![0x78, 0x9c];
        zlib_data.extend(THIS_IS_THIS_TEST);
        zlib_data.extend([0, 0, 0, 0]);
        for src in [&zlib_data[..], &THIS_IS_THIS_TEST] {
            let mut out = Vec::new();
            zlib(&mut out, src, LIMIT).unwrap();
            assert_eq!(out, THIS_IS_THIS_TEXT);
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_limit_holds_and_a_cut_stream_keeps_its_start() {
        let mut out = Vec::new();
        assert_eq!(
            inflate(&mut out, &THIS_IS_THIS_TEST, 10),
            Err(Error::Overflow)
        );
        assert_eq!(out, &THIS_IS_THIS_TEXT[..out.len()]);
        assert!(out.len() <= 10);

        // Cut short, it ends early or fails: either way with its start.
        let mut out = Vec::new();
        let _ = inflate(&mut out, &THIS_IS_THIS_TEST[..12], LIMIT);
        assert!(!out.is_empty() && out.len() < THIS_IS_THIS_TEXT.len());
        assert_eq!(out, &THIS_IS_THIS_TEXT[..out.len()]);

        let stored = [0x01, 0x05, 0x00, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
        let mut out = Vec::new();
        assert_eq!(inflate(&mut out, &stored, 4), Err(Error::Overflow));
        assert!(out.is_empty());

        // A distance of one before any byte.
        assert_eq!(inflated(&[0x03, 0x02]), Err(Error::InvalidDistance));
    }
}

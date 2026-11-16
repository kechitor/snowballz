//! DEFLATE (RFC 1951) / zlib (RFC 1950) decompression, CRC-32 and a tiny
//! zlib *writer* (stored blocks only, used to save screenshots as PNG).

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bit: 0, nbits: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        while self.nbits <= 56 {
            let b = if self.pos < self.data.len() { self.data[self.pos] } else { 0 };
            self.pos += 1;
            self.bit |= (b as u64) << self.nbits;
            self.nbits += 8;
        }
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        if self.nbits < n {
            self.refill();
        }
        if self.pos > self.data.len() + 8 {
            return Err("deflate: unexpected end of data".into());
        }
        let v = (self.bit & ((1u64 << n) - 1)) as u32;
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }

    fn align_byte(&mut self) {
        let r = self.nbits % 8;
        self.bit >>= r;
        self.nbits -= r;
    }

    /// Byte position of the next unread whole byte.
    fn byte_pos(&self) -> usize {
        self.pos - (self.nbits / 8) as usize
    }

    fn reset_to(&mut self, pos: usize) {
        self.pos = pos;
        self.bit = 0;
        self.nbits = 0;
    }
}

/// Canonical Huffman decoding table (fast 9-bit lookup + slow path).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
    fast: Vec<u16>, // (len << 12) | symbol, 0 = not in fast table
}

const FAST_BITS: u32 = 9;

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        // fast table: walk codes in canonical order
        let mut fast = vec![0u16; 1 << FAST_BITS];
        let mut code: u32 = 0;
        let mut idx = 0usize;
        for len in 1..16u32 {
            for _ in 0..counts[len as usize] {
                if len <= FAST_BITS {
                    // reverse code bits
                    let mut rev = 0u32;
                    for b in 0..len {
                        if code & (1 << b) != 0 {
                            rev |= 1 << (len - 1 - b);
                        }
                    }
                    let step = 1u32 << len;
                    let mut r = rev;
                    while r < (1 << FAST_BITS) {
                        fast[r as usize] = ((len as u16) << 12) | symbols[idx];
                        r += step;
                    }
                }
                code += 1;
                idx += 1;
            }
            code <<= 1;
        }
        Ok(Huffman { counts, symbols, fast })
    }

    #[inline]
    fn decode(&self, br: &mut BitReader) -> Result<u16, String> {
        if br.nbits < 16 {
            br.refill();
        }
        let e = self.fast[(br.bit & ((1 << FAST_BITS) - 1)) as usize];
        if e != 0 {
            let len = (e >> 12) as u32;
            br.bit >>= len;
            br.nbits -= len;
            return Ok(e & 0x0fff);
        }
        // slow path (puff style)
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= br.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("deflate: bad Huffman code".into())
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Decompress raw DEFLATE data.  `size_hint` pre-allocates the output.
pub fn inflate(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(size_hint);
    let mut br = BitReader::new(data);
    loop {
        let last = br.bits(1)?;
        let btype = br.bits(2)?;
        match btype {
            0 => {
                br.align_byte();
                let p = br.byte_pos();
                if p + 4 > data.len() {
                    return Err("deflate: truncated stored block".into());
                }
                let len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
                let nlen = u16::from_le_bytes([data[p + 2], data[p + 3]]) as usize;
                if len != (!nlen & 0xffff) {
                    return Err("deflate: bad stored block length".into());
                }
                if p + 4 + len > data.len() {
                    return Err("deflate: truncated stored block".into());
                }
                out.extend_from_slice(&data[p + 4..p + 4 + len]);
                br.reset_to(p + 4 + len);
            }
            1 | 2 => {
                let (lit, dist) = if btype == 1 {
                    let mut l = [0u8; 288];
                    for (i, v) in l.iter_mut().enumerate() {
                        *v = match i {
                            0..=143 => 8,
                            144..=255 => 9,
                            256..=279 => 7,
                            _ => 8,
                        };
                    }
                    (Huffman::new(&l)?, Huffman::new(&[5u8; 30])?)
                } else {
                    let hlit = br.bits(5)? as usize + 257;
                    let hdist = br.bits(5)? as usize + 1;
                    let hclen = br.bits(4)? as usize + 4;
                    const ORDER: [usize; 19] =
                        [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(hclen) {
                        cl[o] = br.bits(3)? as u8;
                    }
                    let clh = Huffman::new(&cl)?;
                    let total = hlit + hdist;
                    let mut lens = vec![0u8; total];
                    let mut i = 0;
                    while i < total {
                        let sym = clh.decode(&mut br)?;
                        let (value, n) = match sym {
                            0..=15 => (sym as u8, 1),
                            16 => {
                                if i == 0 {
                                    return Err("deflate: repeat with no previous length".into());
                                }
                                (lens[i - 1], 3 + br.bits(2)? as usize)
                            }
                            17 => (0, 3 + br.bits(3)? as usize),
                            _ => (0, 11 + br.bits(7)? as usize),
                        };
                        if i + n > total {
                            return Err("deflate: too many code lengths".into());
                        }
                        for l in &mut lens[i..i + n] {
                            *l = value;
                        }
                        i += n;
                    }
                    (Huffman::new(&lens[..hlit])?, Huffman::new(&lens[hlit..])?)
                };
                loop {
                    let sym = lit.decode(&mut br)? as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = sym - 257;
                        if li >= 29 {
                            return Err("deflate: bad length symbol".into());
                        }
                        let len = LEN_BASE[li] as usize + br.bits(LEN_EXTRA[li] as u32)? as usize;
                        let di = dist.decode(&mut br)? as usize;
                        if di >= 30 {
                            return Err("deflate: bad distance symbol".into());
                        }
                        let d = DIST_BASE[di] as usize + br.bits(DIST_EXTRA[di] as u32)? as usize;
                        if d > out.len() {
                            return Err("deflate: distance too far back".into());
                        }
                        let start = out.len() - d;
                        if d >= len {
                            out.extend_from_within(start..start + len);
                        } else {
                            for k in 0..len {
                                let b = out[start + k];
                                out.push(b);
                            }
                        }
                    }
                }
            }
            _ => return Err("deflate: bad block type".into()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

/// Decompress zlib data (2 byte header + deflate + adler32).
pub fn zlib_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    if data.len() < 2 {
        return Err("zlib: too short".into());
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0f != 8 || ((cmf as u16) << 8 | flg as u16) % 31 != 0 {
        return Err("zlib: bad header".into());
    }
    if flg & 0x20 != 0 {
        return Err("zlib: preset dictionary not supported".into());
    }
    inflate(&data[2..], size_hint)
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn crc_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static T: OnceLock<[u32; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, e) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    })
}

pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    let t = crc_table();
    crc = !crc;
    for &b in data {
        crc = t[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

/// zlib stream made of stored (uncompressed) blocks.
pub fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(c) = chunks.next() {
        out.push(if chunks.peek().is_none() { 1 } else { 0 });
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_roundtrip() {
        let data: Vec<u8> = (0..200000u32).map(|i| (i * 7 % 251) as u8).collect();
        let z = zlib_store(&data);
        assert_eq!(zlib_decompress(&z, 0).unwrap(), data);
    }

    #[test]
    fn crc() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E60398);
    }

    #[test]
    fn real_zlib_streams() {
        // made by Python's zlib at levels 0, 1, 6 and 9 (stored, fixed and
        // dynamic Huffman blocks), see tests/data/make_fixtures.py
        let raw = std::fs::read("tests/data/zlib_raw.bin").unwrap();
        for level in [0, 1, 6, 9] {
            let z = std::fs::read(format!("tests/data/zlib_{}.bin", level)).unwrap();
            assert_eq!(zlib_decompress(&z, 0).unwrap(), raw, "level {}", level);
        }
        let fixed = std::fs::read("tests/data/zlib_fixed.bin").unwrap();
        assert_eq!(zlib_decompress(&fixed, 0).unwrap(), b"hello hello hello hello world\n".repeat(20));
    }
}

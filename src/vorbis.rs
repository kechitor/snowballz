//! Ogg Vorbis decoder (Vorbis I), so the music needs no extra libraries on
//! any system.
//!
//! It covers what Vorbis encoders (libvorbis, ffmpeg) produce: floor type 1,
//! residue types 0, 1 and 2, codebook lookup types 1 and 2, any number of
//! channels and both block sizes.  Floor type 0 (not produced by encoders
//! since 2002) is reported as unsupported.  The whole file is kept in memory
//! (the game's music is a few MB).

// ---------------------------------------------------------------------------
// Ogg pages -> packets
// ---------------------------------------------------------------------------

/// Split an Ogg stream (the first logical stream) into packets.  Returns the
/// packets and the last granule position (= total number of samples).
fn ogg_packets(d: &[u8]) -> Result<(Vec<Vec<u8>>, i64), String> {
    let mut pos = 0;
    let mut packets = Vec::new();
    let mut cur = Vec::new();
    let mut last_granule = -1i64;
    let mut serial = None;
    while pos + 27 <= d.len() {
        if &d[pos..pos + 4] != b"OggS" {
            // lost sync: look for the next page
            match d[pos + 1..].windows(4).position(|w| w == b"OggS") {
                Some(p) => {
                    pos += 1 + p;
                    continue;
                }
                None => break,
            }
        }
        let granule = i64::from_le_bytes(d[pos + 6..pos + 14].try_into().unwrap());
        let ser = u32::from_le_bytes(d[pos + 14..pos + 18].try_into().unwrap());
        let nseg = d[pos + 26] as usize;
        if pos + 27 + nseg > d.len() {
            break;
        }
        let table = &d[pos + 27..pos + 27 + nseg];
        let body = pos + 27 + nseg;
        let total: usize = table.iter().map(|&l| l as usize).sum();
        if body + total > d.len() {
            break;
        }
        pos = body + total;
        if *serial.get_or_insert(ser) != ser {
            continue; // another multiplexed stream
        }
        let mut off = body;
        for &l in table {
            cur.extend_from_slice(&d[off..off + l as usize]);
            off += l as usize;
            if l < 255 {
                packets.push(std::mem::take(&mut cur));
            }
        }
        if granule >= 0 {
            last_granule = granule;
        }
    }
    if packets.is_empty() {
        return Err("not an Ogg file".into());
    }
    Ok((packets, last_granule))
}

// ---------------------------------------------------------------------------
// bit reader (Vorbis packs bits LSB first)
// ---------------------------------------------------------------------------

struct Bits<'a> {
    d: &'a [u8],
    pos: usize, // in bits
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0 }
    }

    fn left(&self) -> usize {
        (self.d.len() * 8).saturating_sub(self.pos)
    }

    /// Read `n` (0..=32) bits; None at the end of the packet.
    fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if (n as usize) > self.left() {
            self.pos = self.d.len() * 8;
            return None;
        }
        let mut v = 0u64;
        let mut got = 0u32;
        while got < n {
            let byte = self.d[self.pos >> 3] as u64;
            let off = (self.pos & 7) as u32;
            let take = (8 - off).min(n - got);
            v |= ((byte >> off) & ((1 << take) - 1)) << got;
            got += take;
            self.pos += take as usize;
        }
        Some(v as u32)
    }

    fn flag(&mut self) -> Option<bool> {
        self.read(1).map(|v| v == 1)
    }

    /// The next 32 bits without consuming them (zeros past the end).
    fn peek32(&self) -> u32 {
        let mut v = 0u64;
        let byte0 = self.pos >> 3;
        for i in 0..5 {
            if let Some(&b) = self.d.get(byte0 + i) {
                v |= (b as u64) << (8 * i);
            }
        }
        (v >> (self.pos & 7)) as u32
    }

    fn skip(&mut self, n: u32) -> Option<()> {
        if (n as usize) > self.left() {
            self.pos = self.d.len() * 8;
            return None;
        }
        self.pos += n as usize;
        Some(())
    }
}

fn ilog(x: u32) -> u32 {
    32 - x.leading_zeros()
}

fn float32_unpack(x: u32) -> f32 {
    let mantissa = (x & 0x1f_ffff) as f64;
    let exponent = ((x & 0x7fe0_0000) >> 21) as i32;
    let v = mantissa * 2f64.powi(exponent - 788);
    (if x & 0x8000_0000 != 0 { -v } else { v }) as f32
}

/// The greatest r with r^dims <= entries.
fn lookup1_values(entries: u32, dims: u32) -> u32 {
    let pow_le = |r: u64| -> bool {
        let mut acc: u64 = 1;
        for _ in 0..dims {
            acc = match acc.checked_mul(r) {
                Some(a) if a <= entries as u64 => a,
                _ => return false,
            };
        }
        true
    };
    let mut r = (entries as f64).powf(1.0 / dims.max(1) as f64).floor() as u64;
    while r > 0 && !pow_le(r) {
        r -= 1;
    }
    while pow_le(r + 1) {
        r += 1;
    }
    r as u32
}

// ---------------------------------------------------------------------------
// codebooks
// ---------------------------------------------------------------------------

const FAST_BITS: u32 = 10;

struct Codebook {
    dims: usize,
    /// entry + length for codes up to FAST_BITS long (-1: none)
    fast: Vec<(i32, u8)>,
    /// (code, length, entry) for longer codes
    slow: Vec<(u32, u8, u32)>,
    single: Option<(u32, u8)>,
    vq: Vec<f32>,
}

impl Codebook {
    fn read(b: &mut Bits) -> Result<Codebook, String> {
        let err = || "truncated codebook".to_string();
        if b.read(24).ok_or_else(err)? != 0x564342 {
            return Err("bad codebook sync".into());
        }
        let dims = b.read(16).ok_or_else(err)? as usize;
        let entries = b.read(24).ok_or_else(err)? as usize;
        let ordered = b.flag().ok_or_else(err)?;
        let mut lens = vec![0u8; entries];
        if !ordered {
            let sparse = b.flag().ok_or_else(err)?;
            for l in lens.iter_mut() {
                if !sparse || b.flag().ok_or_else(err)? {
                    *l = b.read(5).ok_or_else(err)? as u8 + 1;
                }
            }
        } else {
            let mut cur = 0usize;
            let mut len = b.read(5).ok_or_else(err)? + 1;
            while cur < entries {
                let n = b.read(ilog((entries - cur) as u32)).ok_or_else(err)? as usize;
                if cur + n > entries || len > 32 {
                    return Err("bad ordered codebook".into());
                }
                for l in &mut lens[cur..cur + n] {
                    *l = len as u8;
                }
                cur += n;
                len += 1;
            }
        }
        let lookup = b.read(4).ok_or_else(err)?;
        let mut vq = Vec::new();
        match lookup {
            0 => {}
            1 | 2 => {
                let min = float32_unpack(b.read(32).ok_or_else(err)?);
                let delta = float32_unpack(b.read(32).ok_or_else(err)?);
                let value_bits = b.read(4).ok_or_else(err)? + 1;
                let sequence = b.flag().ok_or_else(err)?;
                let nvalues = if lookup == 1 {
                    lookup1_values(entries as u32, dims as u32) as usize
                } else {
                    entries * dims
                };
                let mut mult = Vec::with_capacity(nvalues);
                for _ in 0..nvalues {
                    mult.push(b.read(value_bits).ok_or_else(err)? as f32);
                }
                vq = vec![0.0; entries * dims];
                for e in 0..entries {
                    let mut last = 0.0f32;
                    let mut div = 1usize;
                    for i in 0..dims {
                        let off = if lookup == 1 { (e / div) % nvalues.max(1) } else { e * dims + i };
                        let v = mult.get(off).copied().unwrap_or(0.0) * delta + min + last;
                        if sequence {
                            last = v;
                        }
                        vq[e * dims + i] = v;
                        if lookup == 1 {
                            div = div.saturating_mul(nvalues);
                        }
                    }
                }
            }
            _ => return Err("bad codebook lookup type".into()),
        }
        let mut book = Codebook { dims, fast: vec![(-1, 0); 1 << FAST_BITS], slow: Vec::new(), single: None, vq };
        book.build_huffman(&lens)?;
        Ok(book)
    }

    /// Assign the codewords (Vorbis I spec 3.2.1) and build the decode tables.
    fn build_huffman(&mut self, lens: &[u8]) -> Result<(), String> {
        let used: Vec<usize> = (0..lens.len()).filter(|&i| lens[i] > 0).collect();
        if used.is_empty() {
            return Ok(());
        }
        if used.len() == 1 {
            self.single = Some((used[0] as u32, lens[used[0]]));
            return Ok(());
        }
        let mut available = [0u32; 33];
        let add = |entry: usize, code_msb: u32, len: u8, s: &mut Codebook| {
            let code = code_msb.reverse_bits(); // first bit read = lowest bit
            if (len as u32) <= FAST_BITS {
                let step = 1usize << len;
                let mut i = code as usize;
                while i < (1 << FAST_BITS) {
                    s.fast[i] = (entry as i32, len);
                    i += step;
                }
            } else {
                s.slow.push((code, len, entry as u32));
            }
        };
        let first = used[0];
        add(first, 0, lens[first], self);
        for i in 1..=lens[first] as usize {
            available[i] = 1u32.wrapping_shl(32 - i as u32);
        }
        for &e in &used[1..] {
            let len = lens[e] as usize;
            let mut z = len;
            while z > 0 && available[z] == 0 {
                z -= 1;
            }
            if z == 0 {
                return Err("overspecified codebook".into());
            }
            let res = available[z];
            available[z] = 0;
            add(e, res, len as u8, self);
            for y in (z + 1..=len).rev() {
                available[y] = res.wrapping_add(1u32.wrapping_shl(32 - y as u32));
            }
        }
        // longest first is not needed: codes are prefix free
        Ok(())
    }

    fn decode(&self, b: &mut Bits) -> Option<u32> {
        if let Some((e, len)) = self.single {
            b.skip(len as u32)?;
            return Some(e);
        }
        let bits = b.peek32();
        let (e, len) = self.fast[(bits & ((1 << FAST_BITS) - 1)) as usize];
        if e >= 0 {
            b.skip(len as u32)?;
            return Some(e as u32);
        }
        for &(code, len, e) in &self.slow {
            let mask = if len >= 32 { u32::MAX } else { (1u32 << len) - 1 };
            if bits & mask == code {
                b.skip(len as u32)?;
                return Some(e);
            }
        }
        b.pos = b.d.len() * 8; // invalid code: treat as the end of the packet
        None
    }

    fn decode_vq(&self, b: &mut Bits) -> Option<&[f32]> {
        let e = self.decode(b)? as usize;
        self.vq.get(e * self.dims..(e + 1) * self.dims)
    }
}

// ---------------------------------------------------------------------------
// floor type 1
// ---------------------------------------------------------------------------

struct Floor1 {
    partition_class: Vec<u8>,
    class_dims: Vec<u8>,
    class_subclasses: Vec<u8>,
    class_master: Vec<u8>,
    subclass_books: Vec<Vec<i32>>,
    multiplier: u32,
    xs: Vec<u32>,
    /// indices of xs, sorted by x
    sorted: Vec<usize>,
    low: Vec<usize>,
    high: Vec<usize>,
}

impl Floor1 {
    fn read(b: &mut Bits, nbooks: usize) -> Result<Floor1, String> {
        let err = || "truncated floor".to_string();
        let partitions = b.read(5).ok_or_else(err)? as usize;
        let mut partition_class = Vec::new();
        for _ in 0..partitions {
            partition_class.push(b.read(4).ok_or_else(err)? as u8);
        }
        let classes = partition_class.iter().map(|&c| c as usize + 1).max().unwrap_or(0);
        let (mut class_dims, mut class_subclasses, mut class_master, mut subclass_books) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..classes {
            class_dims.push(b.read(3).ok_or_else(err)? as u8 + 1);
            let sub = b.read(2).ok_or_else(err)? as u8;
            class_subclasses.push(sub);
            class_master.push(if sub > 0 { b.read(8).ok_or_else(err)? as u8 } else { 0 });
            let mut books = Vec::new();
            for _ in 0..(1 << sub) {
                books.push(b.read(8).ok_or_else(err)? as i32 - 1);
            }
            subclass_books.push(books);
        }
        let multiplier = b.read(2).ok_or_else(err)? + 1;
        let rangebits = b.read(4).ok_or_else(err)?;
        let mut xs = vec![0, 1u32 << rangebits];
        for &c in &partition_class {
            for _ in 0..class_dims[c as usize] {
                xs.push(b.read(rangebits).ok_or_else(err)?);
            }
        }
        if xs.len() > 65 {
            return Err("too many floor points".into());
        }
        let check = |i: u8| (i as usize) < nbooks;
        if !class_master.iter().zip(&class_subclasses).all(|(&m, &s)| s == 0 || check(m))
            || !subclass_books.iter().flatten().all(|&bk| bk < nbooks as i32)
        {
            return Err("floor uses a missing codebook".into());
        }
        let mut sorted: Vec<usize> = (0..xs.len()).collect();
        sorted.sort_by_key(|&i| xs[i]);
        let (mut low, mut high) = (vec![0; xs.len()], vec![0; xs.len()]);
        for i in 2..xs.len() {
            // the closest smaller and larger x among the earlier points
            let (mut lo, mut hi) = (0usize, 1usize);
            let (mut lx, mut hx) = (-1i64, i64::MAX);
            for j in 0..i {
                let xj = xs[j] as i64;
                if xj < xs[i] as i64 && xj > lx {
                    lx = xj;
                    lo = j;
                }
                if xj > xs[i] as i64 && xj < hx {
                    hx = xj;
                    hi = j;
                }
            }
            low[i] = lo;
            high[i] = hi;
        }
        Ok(Floor1 { partition_class, class_dims, class_subclasses, class_master, subclass_books, multiplier, xs, sorted, low, high })
    }

    /// The packet part of the floor: None if the channel is unused.
    fn decode(&self, b: &mut Bits, books: &[Codebook]) -> Option<Vec<i32>> {
        if !b.flag()? {
            return None;
        }
        let range = [256u32, 128, 86, 64][(self.multiplier - 1) as usize];
        let bits = ilog(range - 1);
        let mut y = vec![0i32; self.xs.len()];
        y[0] = b.read(bits)? as i32;
        y[1] = b.read(bits)? as i32;
        let mut off = 2;
        for &c in &self.partition_class {
            let c = c as usize;
            let cdim = self.class_dims[c] as usize;
            let cbits = self.class_subclasses[c] as u32;
            let csub = (1u32 << cbits) - 1;
            let mut cval = if cbits > 0 { books[self.class_master[c] as usize].decode(b)? } else { 0 };
            for j in 0..cdim {
                let book = self.subclass_books[c][(cval & csub) as usize];
                cval >>= cbits;
                y[off + j] = if book >= 0 { books[book as usize].decode(b)? as i32 } else { 0 };
            }
            off += cdim;
        }
        Some(y)
    }

    /// The floor curve (step 2), multiplied into `v` (n/2 values).
    fn apply(&self, y: &[i32], v: &mut [f32]) {
        let range = [256i32, 128, 86, 64][(self.multiplier - 1) as usize];
        let n = self.xs.len();
        let mut fy = vec![0i32; n];
        let mut used = vec![false; n];
        used[0] = true;
        used[1] = true;
        fy[0] = y[0];
        fy[1] = y[1];
        for i in 2..n {
            let (lo, hi) = (self.low[i], self.high[i]);
            let predicted = render_point(self.xs[lo] as i32, fy[lo], self.xs[hi] as i32, fy[hi], self.xs[i] as i32);
            let val = y[i];
            let highroom = range - predicted;
            let lowroom = predicted;
            let room = if highroom < lowroom { highroom * 2 } else { lowroom * 2 };
            if val != 0 {
                used[lo] = true;
                used[hi] = true;
                used[i] = true;
                fy[i] = if val >= room {
                    if highroom > lowroom {
                        val - lowroom + predicted
                    } else {
                        predicted - val + highroom - 1
                    }
                } else if val & 1 == 1 {
                    predicted - (val + 1) / 2
                } else {
                    predicted + val / 2
                };
            } else {
                used[i] = false;
                fy[i] = predicted;
            }
        }
        let m = self.multiplier as i32;
        let len = v.len() as i32;
        let mut curve = vec![0i32; v.len()];
        let first = self.sorted[0];
        let (mut lx, mut ly) = (0i32, fy[first] * m);
        let mut hy = ly;
        let mut hx = 0i32;
        for &i in &self.sorted[1..] {
            if used[i] {
                hy = fy[i] * m;
                hx = self.xs[i] as i32;
                render_line(lx, ly, hx, hy, &mut curve);
                lx = hx;
                ly = hy;
            }
        }
        if hx < len {
            render_line(hx, hy, len, hy, &mut curve);
        }
        for (s, &c) in v.iter_mut().zip(&curve) {
            *s *= f32::from_bits(INVERSE_DB[c.clamp(0, 255) as usize]);
        }
    }
}

fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    let dy = y1 - y0;
    let adx = (x1 - x0).max(1);
    let err = dy.abs() * (x - x0);
    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

fn render_line(x0: i32, y0: i32, x1: i32, y1: i32, v: &mut [i32]) {
    let n = v.len() as i32;
    let dy = y1 - y0;
    let adx = x1 - x0;
    if adx <= 0 {
        return;
    }
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let ady = dy.abs() - base.abs() * adx;
    let (mut y, mut err) = (y0, 0);
    if x0 < n && x0 >= 0 {
        v[x0 as usize] = y;
    }
    for x in x0 + 1..x1.min(n) {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        if x >= 0 {
            v[x as usize] = y;
        }
    }
}

// ---------------------------------------------------------------------------
// residues
// ---------------------------------------------------------------------------

struct Residue {
    kind: u32,
    begin: usize,
    end: usize,
    partition_size: usize,
    classifications: usize,
    classbook: usize,
    books: Vec<[i32; 8]>,
}

impl Residue {
    fn read(b: &mut Bits, kind: u32, nbooks: usize) -> Result<Residue, String> {
        let err = || "truncated residue".to_string();
        let begin = b.read(24).ok_or_else(err)? as usize;
        let end = b.read(24).ok_or_else(err)? as usize;
        let partition_size = b.read(24).ok_or_else(err)? as usize + 1;
        let classifications = b.read(6).ok_or_else(err)? as usize + 1;
        let classbook = b.read(8).ok_or_else(err)? as usize;
        let mut cascade = Vec::new();
        for _ in 0..classifications {
            let low = b.read(3).ok_or_else(err)?;
            let high = if b.flag().ok_or_else(err)? { b.read(5).ok_or_else(err)? } else { 0 };
            cascade.push(high * 8 + low);
        }
        let mut books = Vec::new();
        for &c in &cascade {
            let mut row = [-1i32; 8];
            for (j, slot) in row.iter_mut().enumerate() {
                if c & (1 << j) != 0 {
                    let bk = b.read(8).ok_or_else(err)? as i32;
                    if bk as usize >= nbooks {
                        return Err("residue uses a missing codebook".into());
                    }
                    *slot = bk;
                }
            }
            books.push(row);
        }
        if classbook >= nbooks {
            return Err("residue uses a missing codebook".into());
        }
        Ok(Residue { kind, begin, end, partition_size, classifications, classbook, books })
    }

    /// Decode into `vecs` (one per channel of the submap, each `n` long).
    fn decode(&self, b: &mut Bits, books: &[Codebook], vecs: &mut [Vec<f32>], skip: &[bool], n: usize) {
        if self.kind == 2 {
            if skip.iter().all(|&s| s) {
                return;
            }
            let ch = vecs.len();
            let mut inter = vec![vec![0.0f32; n * ch]];
            self.decode_01(b, books, &mut inter, &[false], n * ch, 1);
            for (i, &x) in inter[0].iter().enumerate() {
                vecs[i % ch][i / ch] = x;
            }
        } else {
            self.decode_01(b, books, vecs, skip, n, self.kind);
        }
    }

    fn decode_01(&self, b: &mut Bits, books: &[Codebook], vecs: &mut [Vec<f32>], skip: &[bool], size: usize, kind: u32) {
        let begin = self.begin.min(size);
        let end = self.end.min(size);
        let psize = self.partition_size;
        let parts = (end - begin) / psize;
        if parts == 0 {
            return;
        }
        let classbook = &books[self.classbook];
        let per_word = classbook.dims.max(1);
        let ch = vecs.len();
        let mut classes = vec![vec![0usize; parts + per_word]; ch];
        for pass in 0..8 {
            let mut p = 0;
            while p < parts {
                if pass == 0 {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let Some(mut t) = classbook.decode(b) else { return };
                        for i in (0..per_word).rev() {
                            classes[j][p + i] = t as usize % self.classifications;
                            t /= self.classifications as u32;
                        }
                    }
                }
                let mut i = 0;
                while i < per_word && p < parts {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let book = self.books[classes[j][p]][pass];
                        if book < 0 {
                            continue;
                        }
                        let book = &books[book as usize];
                        let off = begin + p * psize;
                        let dims = book.dims.max(1);
                        let v = &mut vecs[j];
                        if kind == 0 {
                            let step = psize / dims;
                            for k in 0..step {
                                let Some(vq) = book.decode_vq(b) else { return };
                                for (d, &x) in vq.iter().enumerate() {
                                    if let Some(s) = v.get_mut(off + k + d * step) {
                                        *s += x;
                                    }
                                }
                            }
                        } else {
                            let mut k = 0;
                            while k < psize {
                                let Some(vq) = book.decode_vq(b) else { return };
                                for &x in vq {
                                    if k < psize {
                                        if let Some(s) = v.get_mut(off + k) {
                                            *s += x;
                                        }
                                    }
                                    k += 1;
                                }
                            }
                        }
                    }
                    i += 1;
                    p += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// inverse MDCT (via a DCT-IV computed with an n/4 point complex FFT)
// ---------------------------------------------------------------------------

struct Mdct {
    n: usize,
    pre: Vec<(f32, f32)>,
    post: Vec<(f32, f32)>,
    tw: Vec<(f32, f32)>,
    rev: Vec<usize>,
}

impl Mdct {
    fn new(n: usize) -> Mdct {
        use std::f64::consts::PI;
        let m = n / 2;
        let h = m / 2;
        let cis = |a: f64| (a.cos() as f32, a.sin() as f32);
        let pre = (0..h).map(|k| cis(-PI * (k as f64 + 0.25) / m as f64)).collect();
        let post = (0..h).map(|k| cis(-PI * k as f64 / m as f64)).collect();
        let tw = (0..h.max(1)).map(|k| cis(-2.0 * PI * k as f64 / h as f64)).collect();
        let bits = h.trailing_zeros();
        let rev = (0..h).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Mdct { n, pre, post, tw, rev }
    }

    /// n/2 coefficients -> n samples.
    fn inverse(&self, x: &[f32], out: &mut [f32]) {
        let m = self.n / 2;
        let h = m / 2;
        let mul = |a: (f32, f32), b: (f32, f32)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        let mut a = vec![(0.0f32, 0.0f32); h];
        for k in 0..h {
            a[self.rev[k]] = mul((x[2 * k], x[m - 1 - 2 * k]), self.pre[k]);
        }
        let mut len = 2;
        while len <= h {
            let half = len / 2;
            let step = h / len;
            for start in (0..h).step_by(len) {
                for j in 0..half {
                    let u = a[start + j];
                    let v = mul(a[start + j + half], self.tw[j * step]);
                    a[start + j] = (u.0 + v.0, u.1 + v.1);
                    a[start + j + half] = (u.0 - v.0, u.1 - v.1);
                }
            }
            len *= 2;
        }
        let mut y = vec![0.0f32; m];
        for k in 0..h {
            let u = mul(a[k], self.post[k]);
            y[2 * k] = u.0;
            y[m - 1 - 2 * k] = -u.1;
        }
        // unfold the DCT-IV into the IMDCT
        for (i, o) in out.iter_mut().enumerate().take(self.n) {
            let j = i + m / 2;
            *o = if j < m {
                y[j]
            } else if j < 2 * m {
                -y[2 * m - 1 - j]
            } else {
                -y[j - 2 * m]
            };
        }
    }
}

// ---------------------------------------------------------------------------
// the decoder
// ---------------------------------------------------------------------------

struct Mapping {
    coupling: Vec<(usize, usize)>,
    mux: Vec<usize>,
    submap_floor: Vec<usize>,
    submap_residue: Vec<usize>,
}

struct Mode {
    long: bool,
    mapping: usize,
}

struct Prev {
    n: usize,
    /// right half of the last windowed block (n/2 samples per channel)
    right: Vec<Vec<f32>>,
    /// where its right window slope starts, counted from n/2
    rs: usize,
}

pub struct Decoder {
    packets: Vec<Vec<u8>>,
    next: usize,
    first_audio: usize,
    total: Option<u64>,
    out_count: u64,
    pub channels: usize,
    pub rate: u32,
    bs: [usize; 2],
    books: Vec<Codebook>,
    floors: Vec<Floor1>,
    residues: Vec<Residue>,
    mappings: Vec<Mapping>,
    modes: Vec<Mode>,
    mdct: [Mdct; 2],
    slopes: [Vec<f32>; 2],
    prev: Option<Prev>,
}

impl Decoder {
    pub fn open(path: &str) -> Result<Decoder, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
        Decoder::new(&data)
    }

    pub fn new(data: &[u8]) -> Result<Decoder, String> {
        let (packets, last_granule) = ogg_packets(data)?;
        if packets.len() < 3 {
            return Err("not an Ogg Vorbis file (missing headers)".into());
        }
        // identification header
        let id = &packets[0];
        if id.len() < 30 || id[0] != 1 || &id[1..7] != b"vorbis" {
            return Err("not an Ogg Vorbis file".into());
        }
        let channels = id[11] as usize;
        let rate = u32::from_le_bytes(id[12..16].try_into().unwrap());
        let bs = [1usize << (id[28] & 15), 1usize << (id[28] >> 4)];
        if channels == 0 || rate == 0 || bs[0] < 64 || bs[1] > 8192 || bs[0] > bs[1] {
            return Err("unsupported Vorbis stream".into());
        }
        if packets[1].first() != Some(&3) {
            return Err("missing Vorbis comment header".into());
        }
        // setup header
        let setup = &packets[2];
        if setup.len() < 7 || setup[0] != 5 || &setup[1..7] != b"vorbis" {
            return Err("missing Vorbis setup header".into());
        }
        let mut b = Bits::new(&setup[7..]);
        let err = || "truncated setup header".to_string();
        let nbooks = b.read(8).ok_or_else(err)? as usize + 1;
        let mut books = Vec::new();
        for _ in 0..nbooks {
            books.push(Codebook::read(&mut b)?);
        }
        let ntime = b.read(6).ok_or_else(err)? + 1;
        for _ in 0..ntime {
            if b.read(16).ok_or_else(err)? != 0 {
                return Err("bad time domain transform".into());
            }
        }
        let nfloors = b.read(6).ok_or_else(err)? + 1;
        let mut floors = Vec::new();
        for _ in 0..nfloors {
            match b.read(16).ok_or_else(err)? {
                1 => floors.push(Floor1::read(&mut b, nbooks)?),
                0 => return Err("Vorbis floor type 0 is not supported".into()),
                _ => return Err("bad floor type".into()),
            }
        }
        let nres = b.read(6).ok_or_else(err)? + 1;
        let mut residues = Vec::new();
        for _ in 0..nres {
            let kind = b.read(16).ok_or_else(err)?;
            if kind > 2 {
                return Err("bad residue type".into());
            }
            residues.push(Residue::read(&mut b, kind, nbooks)?);
        }
        let nmaps = b.read(6).ok_or_else(err)? + 1;
        let mut mappings = Vec::new();
        for _ in 0..nmaps {
            if b.read(16).ok_or_else(err)? != 0 {
                return Err("bad mapping type".into());
            }
            let submaps = if b.flag().ok_or_else(err)? { b.read(4).ok_or_else(err)? as usize + 1 } else { 1 };
            let mut coupling = Vec::new();
            if b.flag().ok_or_else(err)? {
                let steps = b.read(8).ok_or_else(err)? + 1;
                let bits = ilog(channels as u32 - 1);
                for _ in 0..steps {
                    let m = b.read(bits).ok_or_else(err)? as usize;
                    let a = b.read(bits).ok_or_else(err)? as usize;
                    if m == a || m >= channels || a >= channels {
                        return Err("bad channel coupling".into());
                    }
                    coupling.push((m, a));
                }
            }
            if b.read(2).ok_or_else(err)? != 0 {
                return Err("bad mapping".into());
            }
            let mut mux = vec![0usize; channels];
            if submaps > 1 {
                for m in mux.iter_mut() {
                    *m = b.read(4).ok_or_else(err)? as usize;
                    if *m >= submaps {
                        return Err("bad mapping mux".into());
                    }
                }
            }
            let (mut submap_floor, mut submap_residue) = (Vec::new(), Vec::new());
            for _ in 0..submaps {
                b.read(8).ok_or_else(err)?;
                let f = b.read(8).ok_or_else(err)? as usize;
                let r = b.read(8).ok_or_else(err)? as usize;
                if f >= floors.len() || r >= residues.len() {
                    return Err("mapping uses a missing floor or residue".into());
                }
                submap_floor.push(f);
                submap_residue.push(r);
            }
            mappings.push(Mapping { coupling, mux, submap_floor, submap_residue });
        }
        let nmodes = b.read(6).ok_or_else(err)? + 1;
        let mut modes = Vec::new();
        for _ in 0..nmodes {
            let long = b.flag().ok_or_else(err)?;
            b.read(16).ok_or_else(err)?;
            b.read(16).ok_or_else(err)?;
            let mapping = b.read(8).ok_or_else(err)? as usize;
            if mapping >= mappings.len() {
                return Err("mode uses a missing mapping".into());
            }
            modes.push(Mode { long, mapping });
        }
        let slope = |len: usize| -> Vec<f32> {
            use std::f64::consts::FRAC_PI_2;
            (0..len)
                .map(|i| {
                    let s = ((i as f64 + 0.5) / len as f64 * FRAC_PI_2).sin();
                    (FRAC_PI_2 * s * s).sin() as f32
                })
                .collect()
        };
        Ok(Decoder {
            packets,
            next: 3,
            first_audio: 3,
            total: if last_granule > 0 { Some(last_granule as u64) } else { None },
            out_count: 0,
            channels,
            rate,
            bs,
            books,
            floors,
            residues,
            mappings,
            modes,
            mdct: [Mdct::new(bs[0]), Mdct::new(bs[1])],
            slopes: [slope(bs[0] / 2), slope(bs[1] / 2)],
            prev: None,
        })
    }

    /// Back to the start (for looping).
    pub fn restart(&mut self) {
        self.next = self.first_audio;
        self.out_count = 0;
        self.prev = None;
    }

    /// Decode the next packet and append its samples (interleaved, -1..1)
    /// to `out`.  Returns the number of frames added, or None at the end.
    pub fn next_frames(&mut self, out: &mut Vec<f32>) -> Option<usize> {
        loop {
            if self.next >= self.packets.len() {
                return None;
            }
            let i = self.next;
            self.next += 1;
            let Some(block) = self.decode_packet(i) else { continue };
            let frames = self.overlap(block, out);
            if frames > 0 || self.next >= self.packets.len() {
                return Some(frames);
            }
        }
    }

    /// One audio packet -> windowed time-domain block (+ its window bounds).
    fn decode_packet(&self, index: usize) -> Option<(usize, usize, usize, usize, Vec<Vec<f32>>)> {
        let pkt = &self.packets[index];
        let mut b = Bits::new(pkt);
        if b.read(1)? != 0 {
            return None; // not an audio packet
        }
        let mode = self.modes.get(b.read(ilog(self.modes.len() as u32 - 1))? as usize)?;
        let n = self.bs[mode.long as usize];
        let (prev_long, next_long) = if mode.long { (b.flag()?, b.flag()?) } else { (false, false) };
        let map = &self.mappings[mode.mapping];
        let half = n / 2;
        let ch = self.channels;

        // floors (an early end of packet just leaves the channel unused)
        let mut fy: Vec<Option<Vec<i32>>> = Vec::with_capacity(ch);
        for c in 0..ch {
            let floor = &self.floors[map.submap_floor[map.mux[c]]];
            fy.push(floor.decode(&mut b, &self.books));
        }
        let mut nonzero: Vec<bool> = fy.iter().map(|f| f.is_some()).collect();
        for &(m, a) in &map.coupling {
            if nonzero[m] || nonzero[a] {
                nonzero[m] = true;
                nonzero[a] = true;
            }
        }

        // residues
        let mut res = vec![vec![0.0f32; half]; ch];
        for (s, &r) in map.submap_residue.iter().enumerate() {
            let chans: Vec<usize> = (0..ch).filter(|&c| map.mux[c] == s).collect();
            let skip: Vec<bool> = chans.iter().map(|&c| !nonzero[c]).collect();
            let mut vecs: Vec<Vec<f32>> = chans.iter().map(|_| vec![0.0f32; half]).collect();
            self.residues[r].decode(&mut b, &self.books, &mut vecs, &skip, half);
            for (k, &c) in chans.iter().enumerate() {
                res[c] = std::mem::take(&mut vecs[k]);
            }
        }

        // inverse channel coupling
        for &(m, a) in map.coupling.iter().rev() {
            for j in 0..half {
                let (mm, aa) = (res[m][j], res[a][j]);
                let (nm, na) = if mm > 0.0 {
                    if aa > 0.0 {
                        (mm, mm - aa)
                    } else {
                        (mm + aa, mm)
                    }
                } else if aa > 0.0 {
                    (mm, mm + aa)
                } else {
                    (mm - aa, mm)
                };
                res[m][j] = nm;
                res[a][j] = na;
            }
        }

        // floor curves, inverse MDCT, window
        let bs0 = self.bs[0];
        let (ls, le) = if mode.long && !prev_long { (n / 4 - bs0 / 4, n / 4 + bs0 / 4) } else { (0, half) };
        let (rs, re) = if mode.long && !next_long { (n * 3 / 4 - bs0 / 4, n * 3 / 4 + bs0 / 4) } else { (half, n) };
        let lslope = &self.slopes[if le - ls == half { mode.long as usize } else { 0 }];
        let rslope = &self.slopes[if re - rs == half { mode.long as usize } else { 0 }];
        let mdct = &self.mdct[mode.long as usize];
        let mut pcm = Vec::with_capacity(ch);
        for c in 0..ch {
            let mut v = std::mem::take(&mut res[c]);
            match &fy[c] {
                Some(y) => self.floors[map.submap_floor[map.mux[c]]].apply(y, &mut v),
                None => v.iter_mut().for_each(|s| *s = 0.0),
            }
            let mut t = vec![0.0f32; n];
            mdct.inverse(&v, &mut t);
            for (i, s) in t.iter_mut().enumerate() {
                let w = if i < ls {
                    0.0
                } else if i < le {
                    lslope[i - ls]
                } else if i < rs {
                    1.0
                } else if i < re {
                    rslope[rslope.len() - 1 - (i - rs)]
                } else {
                    0.0
                };
                *s *= w;
            }
            pcm.push(t);
        }
        Some((n, ls, le, rs, pcm))
    }

    /// Overlap-add with the previous block; returns the frames written.
    fn overlap(&mut self, block: (usize, usize, usize, usize, Vec<Vec<f32>>), out: &mut Vec<f32>) -> usize {
        let (n, ls, le, rs, pcm) = block;
        let half = n / 2;
        let ch = self.channels;
        let mut frames = 0;
        if let Some(prev) = self.prev.take() {
            let pre = prev.rs.min(prev.right[0].len()); // flat part of the previous block
            let ov = (le - ls).min(prev.right[0].len() - pre);
            let count = pre + ov + (half - le.min(half));
            let start = out.len();
            out.resize(start + count * ch, 0.0);
            for c in 0..ch {
                let r = &prev.right[c];
                let p = &pcm[c];
                let mut k = 0;
                for &x in &r[..pre] {
                    out[start + k * ch + c] = x;
                    k += 1;
                }
                for i in 0..ov {
                    out[start + k * ch + c] = r[pre + i] + p[ls + i];
                    k += 1;
                }
                for &x in &p[le.min(half)..half] {
                    out[start + k * ch + c] = x;
                    k += 1;
                }
            }
            frames = count;
            let _ = prev.n;
        }
        self.prev = Some(Prev { n, right: pcm.into_iter().map(|mut v| v.split_off(half)).collect(), rs: rs - half });
        // trim the end of the stream to the last granule position
        if let Some(total) = self.total {
            let room = total.saturating_sub(self.out_count) as usize;
            if frames > room {
                out.truncate(out.len() - (frames - room) * ch);
                frames = room;
            }
        }
        self.out_count += frames as u64;
        frames
    }
}

/// floor1_inverse_dB_table (Vorbis I spec, as f32 bit patterns)
static INVERSE_DB: [u32; 256] = [
    0x33e4b43e, 0x33f39109, 0x3401b28b, 0x340a203c, 0x34131a23, 0x341ca960, 0x3426d7a7, 0x3431af4b,
    0x343d3b50, 0x34498770, 0x3456a023, 0x346492b8, 0x34736d55, 0x34819f88, 0x348a0bfc, 0x34930493,
    0x349c9269, 0x34a6bf32, 0x34b1953f, 0x34bd1f93, 0x34c969e4, 0x34d680ad, 0x34e47136, 0x34f349a6,
    0x35018c88, 0x3509f7c0, 0x3512ef06, 0x351c7b76, 0x3526a6c0, 0x35317b37, 0x353d03da, 0x35494c5e,
    0x3556613b, 0x35644fb9, 0x357325fc, 0x3581798a, 0x3589e386, 0x3592d97c, 0x359c6485, 0x35a68e52,
    0x35b16133, 0x35bce825, 0x35c92edc, 0x35d641ce, 0x35e42e41, 0x35f30257, 0x3601668f, 0x3609cf4f,
    0x3612c3f5, 0x361c4d98, 0x362675e8, 0x36314732, 0x363ccc74, 0x3649115e, 0x36562265, 0x36640cce,
    0x3672deb8, 0x36815397, 0x3689bb1c, 0x3692ae72, 0x369c36af, 0x36a65d81, 0x36b12d35, 0x36bcb0c7,
    0x36c8f3e4, 0x36d60301, 0x36e3eb60, 0x36f2bb1e, 0x370140a2, 0x3709a6eb, 0x371298f1, 0x371c1fc9,
    0x3726451e, 0x3731133d, 0x373c951e, 0x3748d66f, 0x3755e3a2, 0x3763c9f7, 0x37729789, 0x37812daf,
    0x378992be, 0x37928374, 0x379c08e6, 0x37a62cbe, 0x37b0f947, 0x37bc7979, 0x37c8b8fe, 0x37d5c447,
    0x37e3a892, 0x37f273f8, 0x38011ac0, 0x38097e93, 0x38126df9, 0x381bf206, 0x38261462, 0x3830df56,
    0x383c5dd8, 0x38489b92, 0x3855a4f2, 0x38638733, 0x3872506e, 0x388107d3, 0x38896a6b, 0x38925882,
    0x389bdb2a, 0x38a5fc09, 0x38b0c568, 0x38bc423b, 0x38c87e29, 0x38d585a0, 0x38e365d9, 0x38f22ce8,
    0x3900f4e9, 0x39095646, 0x3912430e, 0x391bc451, 0x3925e3b5, 0x3930ab7f, 0x393c26a2, 0x394860c5,
    0x39556653, 0x39634483, 0x39720968, 0x3980e201, 0x39894224, 0x39922d9d, 0x399bad7b, 0x39a5cb63,
    0x39b09199, 0x39bc0b0d, 0x39c84366, 0x39d5470b, 0x39e32332, 0x39f1e5ed, 0x3a00cf1d, 0x3a092e05,
    0x3a121830, 0x3a1b96a9, 0x3a25b315, 0x3a3077b7, 0x3a3bef7c, 0x3a48260a, 0x3a5527c7, 0x3a6301e6,
    0x3a71c278, 0x3a80bc3b, 0x3a8919e9, 0x3a9202c6, 0x3a9b7fdb, 0x3aa59acb, 0x3ab05dd8, 0x3abbd3ef,
    0x3ac808b3, 0x3ad50888, 0x3ae2e09f, 0x3af19f07, 0x3b00a95c, 0x3b0905d0, 0x3b11ed5e, 0x3b1b690f,
    0x3b258284, 0x3b3043fd, 0x3b3bb867, 0x3b47eb61, 0x3b54e94d, 0x3b62bf5d, 0x3b717b9c, 0x3b80967f,
    0x3b88f1ba, 0x3b91d7f9, 0x3b9b5247, 0x3ba56a41, 0x3bb02a27, 0x3bbb9ce2, 0x3bc7ce12, 0x3bd4ca17,
    0x3be29e20, 0x3bf15835, 0x3c0083a6, 0x3c08dda7, 0x3c11c298, 0x3c1b3b82, 0x3c255201, 0x3c301054,
    0x3c3b8161, 0x3c47b0c8, 0x3c54aae5, 0x3c627ce8, 0x3c7134d4, 0x3c8070cf, 0x3c88c996, 0x3c91ad3a,
    0x3c9b24c0, 0x3ca539c5, 0x3caff685, 0x3cbb65e5, 0x3cc79382, 0x3cd48bb9, 0x3ce25bb4, 0x3cf11179,
    0x3d005dfb, 0x3d08b589, 0x3d1197df, 0x3d1b0e02, 0x3d25218d, 0x3d2fdcb9, 0x3d3b4a6d, 0x3d477640,
    0x3d546c91, 0x3d623a85, 0x3d70ee22, 0x3d804b2a, 0x3d88a17f, 0x3d918288, 0x3d9af748, 0x3da50958,
    0x3dafc2f2, 0x3dbb2ef8, 0x3dc75903, 0x3dd44d6d, 0x3de2195c, 0x3df0cad1, 0x3e00385b, 0x3e088d77,
    0x3e116d33, 0x3e1ae090, 0x3e24f127, 0x3e2fa92e, 0x3e3b1387, 0x3e473bca, 0x3e542e4d, 0x3e61f837,
    0x3e70a784, 0x3e80258f, 0x3e887973, 0x3e9157e2, 0x3e9ac9dc, 0x3ea4d8f9, 0x3eaf8f6d, 0x3ebaf81b,
    0x3ec71e95, 0x3ed40f33, 0x3ee1d717, 0x3ef0843d, 0x3f0012c6, 0x3f086572, 0x3f114293, 0x3f1ab32b,
    0x3f24c0ce, 0x3f2f75b1, 0x3f3adcb2, 0x3f470165, 0x3f53f01d, 0x3f61b5fb, 0x3f7060fb, 0x3f800000,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdct_matches_formula() {
        for &n in &[256usize, 2048] {
            let m = n / 2;
            let x: Vec<f32> = (0..m).map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0).collect();
            let mut fast = vec![0.0; n];
            Mdct::new(n).inverse(&x, &mut fast);
            for i in (0..n).step_by(37) {
                let mut s = 0.0f64;
                for (k, &xk) in x.iter().enumerate() {
                    let a = 2.0 * std::f64::consts::PI / n as f64 * (i as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5);
                    s += xk as f64 * a.cos();
                }
                assert!((s - fast[i] as f64).abs() < 1e-2 * (1.0 + s.abs()), "n={} i={} {} vs {}", n, i, s, fast[i]);
            }
        }
    }

    #[test]
    fn decodes_the_game_music() {
        for name in ["main_menu.ogg", "snowballz2.ogg", "llamas_y_bakunas.ogg"] {
            let path = format!("data/music/{}", name);
            let mut d = Decoder::open(&path).unwrap();
            let mut out = Vec::new();
            let mut frames = 0;
            while let Some(f) = d.next_frames(&mut out) {
                frames += f;
            }
            assert_eq!(Some(frames as u64), d.total, "{}", name);
            let peak = out.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
            assert!(peak > 0.1 && peak < 1.5, "{} peak {}", name, peak);
            if let Ok(dir) = std::env::var("VORBIS_DUMP") {
                let bytes: Vec<u8> = out.iter().flat_map(|x| x.to_le_bytes()).collect();
                std::fs::write(format!("{}/{}.f32", dir, name), bytes).unwrap();
            }
        }
    }
}

//! Baseline JPEG decoder (Huffman, 8-bit, any sampling factors, restart
//! markers; greyscale or YCbCr).  Progressive JPEGs are not supported.

use crate::image::Image;

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27,
    20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58,
    59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Default)]
struct Huff {
    // canonical decoding: for each code length the max code and value index
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    vals: Vec<u8>,
    // 8-bit lookahead: (length << 8) | value, 0 = slow path
    look: Vec<u16>,
}

impl Huff {
    fn new(counts: &[u8; 16], vals: Vec<u8>) -> Huff {
        let mut h = Huff { vals, look: vec![0; 256], ..Default::default() };
        let mut code = 0i32;
        let mut k = 0i32;
        for l in 1..=16 {
            let n = counts[l - 1] as i32;
            h.valptr[l] = k;
            h.mincode[l] = code;
            code += n;
            k += n;
            h.maxcode[l] = if n > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        // lookahead table for codes of up to 8 bits
        let mut code = 0u32;
        let mut idx = 0usize;
        for l in 1..=8u32 {
            for _ in 0..counts[l as usize - 1] {
                let shift = 8 - l;
                for fill in 0..(1u32 << shift) {
                    let i = ((code << shift) | fill) as usize;
                    h.look[i] = ((l as u16) << 8) | h.vals[idx] as u16;
                }
                code += 1;
                idx += 1;
            }
            code <<= 1;
        }
        h
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
    hit_marker: bool,
}

impl<'a> Bits<'a> {
    fn fill(&mut self) {
        while self.n <= 24 {
            let mut b = 0u32;
            if !self.hit_marker && self.pos < self.data.len() {
                let c = self.data[self.pos];
                if c == 0xff {
                    let next = *self.data.get(self.pos + 1).unwrap_or(&0);
                    if next == 0 {
                        self.pos += 2;
                        b = 0xff;
                    } else {
                        self.hit_marker = true;
                    }
                } else {
                    self.pos += 1;
                    b = c as u32;
                }
            }
            self.acc |= b << (24 - self.n);
            self.n += 8;
        }
    }

    fn peek(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.fill();
        }
        self.acc >> (32 - k)
    }

    fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }

    fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }

    fn decode(&mut self, h: &Huff) -> Result<u8, String> {
        let p = self.peek(8) as usize;
        let e = h.look[p];
        if e != 0 {
            self.skip((e >> 8) as u32);
            return Ok(e as u8);
        }
        let mut code = 0i32;
        for l in 1..=16 {
            code = (code << 1) | self.get(1) as i32;
            if code <= h.maxcode[l] {
                let i = h.valptr[l] + code - h.mincode[l];
                return h.vals.get(i as usize).copied().ok_or_else(|| "jpeg: bad code".to_string());
            }
        }
        Err("jpeg: bad Huffman code".into())
    }

    fn receive_extend(&mut self, s: u8) -> i32 {
        if s == 0 {
            return 0;
        }
        let v = self.get(s as u32) as i32;
        if v < (1 << (s - 1)) {
            v - (1 << s) + 1
        } else {
            v
        }
    }

    /// Skip to just after the next RSTn marker.
    fn restart(&mut self) {
        self.acc = 0;
        self.n = 0;
        self.hit_marker = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xff && (0xd0..=0xd7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

#[derive(Clone, Default)]
struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    td: usize,
    ta: usize,
    pred: i32,
    plane: Vec<u8>,
    pw: usize,
    ph: usize,
}

fn idct_table() -> &'static [[f32; 8]; 8] {
    use std::sync::OnceLock;
    static T: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0f32; 8]; 8];
        for (x, row) in t.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = cu * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos() / 2.0;
            }
        }
        t
    })
}

fn idct(coef: &[i32; 64], out: &mut [u8], stride: usize) {
    let t = idct_table();
    let mut tmp = [0f32; 64];
    // rows
    for y in 0..8 {
        let row = &coef[y * 8..y * 8 + 8];
        if row[1..].iter().all(|&c| c == 0) {
            let v = row[0] as f32 * t[0][0];
            for x in 0..8 {
                tmp[y * 8 + x] = v;
            }
            continue;
        }
        for x in 0..8 {
            let mut s = 0f32;
            for u in 0..8 {
                s += t[x][u] * row[u] as f32;
            }
            tmp[y * 8 + x] = s;
        }
    }
    // columns
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0f32;
            for v in 0..8 {
                s += t[y][v] * tmp[v * 8 + x];
            }
            let p = (s + 128.0).round();
            out[y * stride + x] = p.clamp(0.0, 255.0) as u8;
        }
    }
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    let mut qt = [[0u16; 64]; 4];
    let mut dc: Vec<Huff> = vec![Huff::default(); 4];
    let mut ac: Vec<Huff> = vec![Huff::default(); 4];
    let mut comps: Vec<Comp> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let mut restart = 0usize;
    let mut adobe_transform: Option<u8> = None;
    let mut pos = 2;
    let mut decoded = false;
    let rd16 = |p: usize| -> usize { ((data[p] as usize) << 8) | data[p + 1] as usize };

    while pos + 4 <= data.len() {
        if data[pos] != 0xff {
            pos += 1;
            continue;
        }
        let marker = data[pos + 1];
        pos += 2;
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 || marker == 0xff {
            if marker == 0xff {
                pos -= 1;
            }
            continue;
        }
        if marker == 0xd9 {
            break;
        }
        let len = rd16(pos);
        let seg_end = pos + len;
        if seg_end > data.len() {
            return Err("jpeg: truncated segment".into());
        }
        let seg = &data[pos + 2..seg_end];
        match marker {
            0xdb => {
                let mut p = 0;
                while p < seg.len() {
                    let pq = seg[p] >> 4;
                    let tq = (seg[p] & 15) as usize & 3;
                    p += 1;
                    for i in 0..64 {
                        qt[tq][ZIGZAG[i]] = if pq == 0 {
                            let v = seg[p] as u16;
                            p += 1;
                            v
                        } else {
                            let v = ((seg[p] as u16) << 8) | seg[p + 1] as u16;
                            p += 2;
                            v
                        };
                    }
                }
            }
            0xc4 => {
                let mut p = 0;
                while p + 17 <= seg.len() {
                    let tc = seg[p] >> 4;
                    let th = (seg[p] & 15) as usize & 3;
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[p + 1..p + 17]);
                    let total: usize = counts.iter().map(|&c| c as usize).sum();
                    p += 17;
                    if p + total > seg.len() {
                        return Err("jpeg: bad DHT".into());
                    }
                    let vals = seg[p..p + total].to_vec();
                    p += total;
                    let h = Huff::new(&counts, vals);
                    if tc == 0 {
                        dc[th] = h;
                    } else {
                        ac[th] = h;
                    }
                }
            }
            0xc0 | 0xc1 => {
                if seg[0] != 8 {
                    return Err("jpeg: only 8-bit JPEGs are supported".into());
                }
                height = rd16(pos + 3);
                width = rd16(pos + 5);
                let n = seg[5] as usize;
                comps.clear();
                for i in 0..n {
                    let b = &seg[6 + i * 3..9 + i * 3];
                    comps.push(Comp {
                        id: b[0],
                        h: (b[1] >> 4).max(1) as usize,
                        v: (b[1] & 15).max(1) as usize,
                        tq: (b[2] & 3) as usize,
                        ..Default::default()
                    });
                }
            }
            0xc2 | 0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Err("jpeg: progressive / lossless JPEGs are not supported".into());
            }
            0xdd => restart = rd16(pos + 2),
            0xee => {
                if seg.len() >= 12 && &seg[..5] == b"Adobe" {
                    adobe_transform = Some(seg[11]);
                }
            }
            0xda => {
                if comps.is_empty() || width == 0 || height == 0 {
                    return Err("jpeg: scan before frame header".into());
                }
                let ns = seg[0] as usize;
                let mut scomps = Vec::new();
                for i in 0..ns {
                    let cid = seg[1 + i * 2];
                    let t = seg[2 + i * 2];
                    if let Some(ci) = comps.iter().position(|c| c.id == cid) {
                        comps[ci].td = (t >> 4) as usize & 3;
                        comps[ci].ta = (t & 15) as usize & 3;
                        scomps.push(ci);
                    }
                }
                let hmax = comps.iter().map(|c| c.h).max().unwrap();
                let vmax = comps.iter().map(|c| c.v).max().unwrap();
                let mcux = width.div_ceil(8 * hmax);
                let mcuy = height.div_ceil(8 * vmax);
                for c in comps.iter_mut() {
                    c.pw = mcux * c.h * 8;
                    c.ph = mcuy * c.v * 8;
                    c.plane = vec![0; c.pw * c.ph];
                    c.pred = 0;
                }
                let mut bits = Bits { data, pos: seg_end, acc: 0, n: 0, hit_marker: false };
                let mut coef = [0i32; 64];
                let single = scomps.len() == 1;
                let (bx_count, by_count) = if single {
                    let c = &comps[scomps[0]];
                    ((width * c.h).div_ceil(8 * hmax), (height * c.v).div_ceil(8 * vmax))
                } else {
                    (mcux, mcuy)
                };
                let total = bx_count * by_count;
                for m in 0..total {
                    if restart > 0 && m > 0 && m % restart == 0 {
                        bits.restart();
                        for c in comps.iter_mut() {
                            c.pred = 0;
                        }
                    }
                    let (mx, my) = (m % bx_count, m / bx_count);
                    for &ci in &scomps {
                        let (bh, bv) = if single { (1, 1) } else { (comps[ci].h, comps[ci].v) };
                        for by in 0..bv {
                            for bx in 0..bh {
                                coef.iter_mut().for_each(|c| *c = 0);
                                let c = &comps[ci];
                                let t = bits.decode(&dc[c.td])?;
                                let diff = bits.receive_extend(t);
                                let q = &qt[c.tq];
                                let pred = c.pred + diff;
                                coef[0] = pred * q[0] as i32;
                                let mut k = 1;
                                while k < 64 {
                                    let rs = bits.decode(&ac[c.ta])?;
                                    let r = (rs >> 4) as usize;
                                    let s = rs & 15;
                                    if s == 0 {
                                        if r == 15 {
                                            k += 16;
                                            continue;
                                        }
                                        break;
                                    }
                                    k += r;
                                    if k > 63 {
                                        break;
                                    }
                                    coef[ZIGZAG[k]] = bits.receive_extend(s) * q[ZIGZAG[k]] as i32;
                                    k += 1;
                                }
                                let c = &mut comps[ci];
                                c.pred = pred;
                                let (x8, y8) = if single {
                                    (mx * 8, my * 8)
                                } else {
                                    ((mx * c.h + bx) * 8, (my * c.v + by) * 8)
                                };
                                if x8 + 8 <= c.pw && y8 + 8 <= c.ph {
                                    let stride = c.pw;
                                    idct(&coef, &mut c.plane[y8 * stride + x8..], stride);
                                }
                            }
                        }
                    }
                }
                decoded = true;
                pos = bits.pos;
                continue;
            }
            _ => {}
        }
        pos = seg_end;
    }
    if !decoded {
        return Err("jpeg: no image data".into());
    }

    // colour conversion (with simple upsampling)
    let hmax = comps.iter().map(|c| c.h).max().unwrap();
    let vmax = comps.iter().map(|c| c.v).max().unwrap();
    let mut img = Image::new(width, height);
    let sample = |c: &Comp, x: usize, y: usize| -> f32 {
        let sx = x * c.h / hmax;
        let sy = y * c.v / vmax;
        c.plane[sy.min(c.ph - 1) * c.pw + sx.min(c.pw - 1)] as f32
    };
    let ycc = comps.len() == 3 && adobe_transform != Some(0);
    for y in 0..height {
        for x in 0..width {
            let px = if comps.len() >= 3 {
                let a = sample(&comps[0], x, y);
                let b = sample(&comps[1], x, y);
                let c = sample(&comps[2], x, y);
                if ycc {
                    let (cb, cr) = (b - 128.0, c - 128.0);
                    let r = a + 1.402 * cr;
                    let g = a - 0.344136 * cb - 0.714136 * cr;
                    let bl = a + 1.772 * cb;
                    [
                        r.round().clamp(0.0, 255.0) as u8,
                        g.round().clamp(0.0, 255.0) as u8,
                        bl.round().clamp(0.0, 255.0) as u8,
                        255,
                    ]
                } else {
                    [a as u8, b as u8, c as u8, 255]
                }
            } else {
                let g = sample(&comps[0], x, y) as u8;
                [g, g, g, 255]
            };
            img.set(x, y, px);
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pil_closely() {
        let img = Image::load("tests/data/bg_menu.jpg").unwrap();
        assert_eq!((img.w, img.h), (3440, 1440));
        let want = std::fs::read("tests/data/bg_menu_crop.rgb").unwrap();
        let crop = img.crop(1000, 400, 256, 256);
        let mut worst = 0i32;
        let mut total = 0i64;
        for i in 0..256 * 256 {
            for c in 0..3 {
                let d = (crop.px[i * 4 + c] as i32 - want[i * 3 + c] as i32).abs();
                worst = worst.max(d);
                total += d as i64;
            }
        }
        let mean = total as f64 / (256.0 * 256.0 * 3.0);
        assert!(worst <= 6 && mean < 0.8, "worst {} mean {}", worst, mean);
    }
}

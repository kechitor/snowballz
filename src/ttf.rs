//! A small TrueType font reader and anti-aliased rasteriser (glyf outlines,
//! cmap formats 4 and 12, simple and composite glyphs).  Metrics follow
//! SDL_ttf (which pygame used), so text keeps its old size and position.

#[derive(Clone)]
pub struct Font {
    data: Vec<u8>,
    pub units_per_em: f32,
    pub ascender: f32,
    pub descender: f32,
    pub line_gap: f32,
    num_hmetrics: usize,
    num_glyphs: usize,
    hmtx: usize,
    loca: usize,
    glyf: usize,
    long_loca: bool,
    cmap: usize,
    cmap_format: u16,
}

/// A rasterised glyph: coverage 0..255, placed at
/// (pen_x + left, baseline + top).
#[derive(Clone, Default)]
pub struct Glyph {
    pub w: usize,
    pub h: usize,
    pub left: i32,
    pub top: i32,
    pub advance: i32,
    pub coverage: Vec<u8>,
}

fn u16_at(d: &[u8], p: usize) -> u16 {
    if p + 2 > d.len() {
        return 0;
    }
    u16::from_be_bytes([d[p], d[p + 1]])
}
fn i16_at(d: &[u8], p: usize) -> i16 {
    u16_at(d, p) as i16
}
fn u32_at(d: &[u8], p: usize) -> u32 {
    if p + 4 > d.len() {
        return 0;
    }
    u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

impl Font {
    pub fn load(path: &str) -> Result<Font, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
        Font::from_bytes(data).map_err(|e| format!("{}: {}", path, e))
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Font, String> {
        let n = u16_at(&data, 4) as usize;
        let mut tables = std::collections::HashMap::new();
        for i in 0..n {
            let p = 12 + i * 16;
            tables.insert(data[p..p + 4].to_vec(), u32_at(&data, p + 8) as usize);
        }
        let t = |name: &[u8]| -> Result<usize, String> {
            tables.get(name).copied().ok_or_else(|| format!("font has no {} table", String::from_utf8_lossy(name)))
        };
        let head = t(b"head")?;
        let hhea = t(b"hhea")?;
        let maxp = t(b"maxp")?;
        let cmap_t = t(b"cmap")?;
        let mut font = Font {
            units_per_em: u16_at(&data, head + 18) as f32,
            long_loca: i16_at(&data, head + 50) != 0,
            ascender: i16_at(&data, hhea + 4) as f32,
            descender: i16_at(&data, hhea + 6) as f32,
            line_gap: i16_at(&data, hhea + 8) as f32,
            num_hmetrics: u16_at(&data, hhea + 34) as usize,
            num_glyphs: u16_at(&data, maxp + 4) as usize,
            hmtx: t(b"hmtx")?,
            loca: t(b"loca")?,
            glyf: t(b"glyf")?,
            cmap: 0,
            cmap_format: 0,
            data: Vec::new(),
        };
        // pick a unicode cmap subtable (prefer full unicode, format 12)
        let nsub = u16_at(&data, cmap_t + 2) as usize;
        let mut best: Option<(i32, usize, u16)> = None;
        for i in 0..nsub {
            let p = cmap_t + 4 + i * 8;
            let pid = u16_at(&data, p);
            let eid = u16_at(&data, p + 2);
            let off = cmap_t + u32_at(&data, p + 4) as usize;
            let fmt = u16_at(&data, off);
            let score = match (pid, eid, fmt) {
                (3, 10, 12) | (0, 4, 12) => 3,
                (3, 1, 4) | (0, _, 4) => 2,
                (3, 0, 4) => 1,
                _ => 0,
            };
            if score > 0 && best.map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, off, fmt));
            }
        }
        let (_, off, fmt) = best.ok_or("font has no usable cmap")?;
        font.cmap = off;
        font.cmap_format = fmt;
        font.data = data;
        if font.units_per_em <= 0.0 {
            font.units_per_em = 1000.0;
        }
        Ok(font)
    }

    pub fn glyph_index(&self, ch: char) -> usize {
        let c = ch as u32;
        let d = &self.data;
        let base = self.cmap;
        let g = if self.cmap_format == 4 {
            if c > 0xffff {
                return 0;
            }
            let segx2 = u16_at(d, base + 6) as usize;
            let ends = base + 14;
            let starts = ends + segx2 + 2;
            let deltas = starts + segx2;
            let ranges = deltas + segx2;
            let mut found = 0usize;
            for s in 0..segx2 / 2 {
                let end = u16_at(d, ends + s * 2) as u32;
                if c <= end {
                    let start = u16_at(d, starts + s * 2) as u32;
                    if c < start {
                        break;
                    }
                    let delta = u16_at(d, deltas + s * 2) as u32;
                    let ro = u16_at(d, ranges + s * 2) as usize;
                    found = if ro == 0 {
                        ((c + delta) & 0xffff) as usize
                    } else {
                        let p = ranges + s * 2 + ro + (c - start) as usize * 2;
                        let gi = u16_at(d, p) as u32;
                        if gi == 0 {
                            0
                        } else {
                            ((gi + delta) & 0xffff) as usize
                        }
                    };
                    break;
                }
            }
            found
        } else {
            let n = u32_at(d, base + 12) as usize;
            let mut found = 0;
            for i in 0..n {
                let p = base + 16 + i * 12;
                let s = u32_at(d, p);
                let e = u32_at(d, p + 4);
                if c >= s && c <= e {
                    found = (u32_at(d, p + 8) + c - s) as usize;
                    break;
                }
            }
            found
        };
        if g < self.num_glyphs {
            g
        } else {
            0
        }
    }

    pub fn advance_units(&self, gid: usize) -> f32 {
        let i = gid.min(self.num_hmetrics.saturating_sub(1));
        u16_at(&self.data, self.hmtx + i * 4) as f32
    }

    fn glyph_range(&self, gid: usize) -> Option<(usize, usize)> {
        if gid >= self.num_glyphs {
            return None;
        }
        let d = &self.data;
        let (a, b) = if self.long_loca {
            (u32_at(d, self.loca + gid * 4) as usize, u32_at(d, self.loca + gid * 4 + 4) as usize)
        } else {
            (u16_at(d, self.loca + gid * 2) as usize * 2, u16_at(d, self.loca + gid * 2 + 2) as usize * 2)
        };
        if b <= a {
            None
        } else {
            Some((self.glyf + a, self.glyf + b))
        }
    }

    /// Contours of (x, y, on_curve) in font units, y up.
    fn outline(&self, gid: usize, depth: u32) -> Vec<Vec<(f32, f32, bool)>> {
        let mut out = Vec::new();
        let Some((start, end)) = self.glyph_range(gid) else { return out };
        let d = &self.data[..end.min(self.data.len())];
        let nc = i16_at(d, start);
        if nc >= 0 {
            let nc = nc as usize;
            let mut p = start + 10;
            let mut ends = Vec::with_capacity(nc);
            for _ in 0..nc {
                ends.push(u16_at(d, p) as usize);
                p += 2;
            }
            let npts = ends.last().map(|e| e + 1).unwrap_or(0);
            let ilen = u16_at(d, p) as usize;
            p += 2 + ilen;
            let mut flags = Vec::with_capacity(npts);
            while flags.len() < npts && p < d.len() {
                let f = d[p];
                p += 1;
                flags.push(f);
                if f & 8 != 0 && p < d.len() {
                    let r = d[p];
                    p += 1;
                    for _ in 0..r {
                        flags.push(f);
                    }
                }
            }
            flags.truncate(npts);
            let mut xs = Vec::with_capacity(npts);
            let mut v = 0i32;
            for &f in &flags {
                if f & 2 != 0 {
                    let dx = *d.get(p).unwrap_or(&0) as i32;
                    p += 1;
                    v += if f & 16 != 0 { dx } else { -dx };
                } else if f & 16 == 0 {
                    v += i16_at(d, p) as i32;
                    p += 2;
                }
                xs.push(v);
            }
            let mut ys = Vec::with_capacity(npts);
            v = 0;
            for &f in &flags {
                if f & 4 != 0 {
                    let dy = *d.get(p).unwrap_or(&0) as i32;
                    p += 1;
                    v += if f & 32 != 0 { dy } else { -dy };
                } else if f & 32 == 0 {
                    v += i16_at(d, p) as i32;
                    p += 2;
                }
                ys.push(v);
            }
            let mut s = 0;
            for &e in &ends {
                let mut c = Vec::new();
                for i in s..=e.min(npts.saturating_sub(1)) {
                    if i < flags.len() {
                        c.push((xs[i] as f32, ys[i] as f32, flags[i] & 1 != 0));
                    }
                }
                if !c.is_empty() {
                    out.push(c);
                }
                s = e + 1;
            }
        } else if depth < 8 {
            // composite glyph
            let mut p = start + 10;
            loop {
                let flags = u16_at(d, p);
                let glyph = u16_at(d, p + 2) as usize;
                p += 4;
                let (a1, a2) = if flags & 1 != 0 {
                    let r = (i16_at(d, p) as f32, i16_at(d, p + 2) as f32);
                    p += 4;
                    r
                } else {
                    let r = (d[p] as i8 as f32, d[p + 1] as i8 as f32);
                    p += 2;
                    r
                };
                let f2 = |v: i16| v as f32 / 16384.0;
                let (mut a, mut b, mut c, mut dd) = (1.0, 0.0, 0.0, 1.0);
                if flags & 8 != 0 {
                    a = f2(i16_at(d, p));
                    dd = a;
                    p += 2;
                } else if flags & 0x40 != 0 {
                    a = f2(i16_at(d, p));
                    dd = f2(i16_at(d, p + 2));
                    p += 4;
                } else if flags & 0x80 != 0 {
                    a = f2(i16_at(d, p));
                    b = f2(i16_at(d, p + 2));
                    c = f2(i16_at(d, p + 4));
                    dd = f2(i16_at(d, p + 6));
                    p += 8;
                }
                let (dx, dy) = if flags & 2 != 0 { (a1, a2) } else { (0.0, 0.0) };
                for contour in self.outline(glyph, depth + 1) {
                    out.push(
                        contour
                            .into_iter()
                            .map(|(x, y, on)| (a * x + c * y + dx, b * x + dd * y + dy, on))
                            .collect(),
                    );
                }
                if flags & 0x20 == 0 {
                    break;
                }
            }
        }
        out
    }

    pub fn scale_for(&self, pixel_size: f32) -> f32 {
        pixel_size / self.units_per_em
    }

    /// Rasterise glyph `gid` at `scale` pixels per font unit.
    pub fn rasterize(&self, gid: usize, scale: f32) -> Glyph {
        let advance = (self.advance_units(gid) * scale).round() as i32;
        let contours = self.outline(gid, 0);
        let mut lines: Vec<(f32, f32, f32, f32)> = Vec::new();
        for c in &contours {
            flatten(c, scale, &mut lines);
        }
        if lines.is_empty() {
            return Glyph { advance, ..Default::default() };
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x0, y0, x1, y1) in &lines {
            minx = minx.min(x0).min(x1);
            maxx = maxx.max(x0).max(x1);
            miny = miny.min(y0).min(y1);
            maxy = maxy.max(y0).max(y1);
        }
        let ox = minx.floor();
        let oy = miny.floor();
        let w = (maxx.ceil() - ox) as usize + 1;
        let h = (maxy.ceil() - oy) as usize + 1;
        let mut acc = vec![0f32; w * h + w + 4];
        for &(x0, y0, x1, y1) in &lines {
            draw_line(&mut acc, w, h, x0 - ox, y0 - oy, x1 - ox, y1 - oy);
        }
        let mut coverage = vec![0u8; w * h];
        let mut a = 0f32;
        for (i, c) in coverage.iter_mut().enumerate() {
            a += acc[i];
            *c = (a.abs().min(1.0) * 255.0 + 0.5) as u8;
        }
        Glyph { w, h, left: ox as i32, top: oy as i32, advance, coverage }
    }
}

/// Outline -> line segments in pixels (y down).
fn flatten(c: &[(f32, f32, bool)], scale: f32, out: &mut Vec<(f32, f32, f32, f32)>) {
    let n = c.len();
    if n < 2 {
        return;
    }
    let pt = |i: usize| -> (f32, f32, bool) {
        let (x, y, on) = c[i % n];
        (x * scale, -y * scale, on)
    };
    // starting point: an on-curve point, or the middle of two off-curve ones
    let first_on = (0..n).find(|&i| c[i].2);
    let (start, sx, sy) = match first_on {
        Some(i) => {
            let p = pt(i);
            (i, p.0, p.1)
        }
        None => {
            let a = pt(0);
            let b = pt(1);
            (0, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
        }
    };
    let (mut cx, mut cy) = (sx, sy);
    let mut ctrl: Option<(f32, f32)> = None;
    let steps = n;
    for k in 1..=steps {
        let (x, y, on) = pt(start + k);
        let (x, y) = if k == steps && first_on.is_some() { (sx, sy) } else { (x, y) };
        let on = on || (k == steps && first_on.is_some());
        if on {
            match ctrl.take() {
                Some((qx, qy)) => quad(out, cx, cy, qx, qy, x, y),
                None => out.push((cx, cy, x, y)),
            }
            cx = x;
            cy = y;
        } else {
            if let Some((qx, qy)) = ctrl {
                let mx = (qx + x) / 2.0;
                let my = (qy + y) / 2.0;
                quad(out, cx, cy, qx, qy, mx, my);
                cx = mx;
                cy = my;
            }
            ctrl = Some((x, y));
        }
    }
    if let Some((qx, qy)) = ctrl {
        quad(out, cx, cy, qx, qy, sx, sy);
    } else if (cx, cy) != (sx, sy) {
        out.push((cx, cy, sx, sy));
    }
}

fn quad(out: &mut Vec<(f32, f32, f32, f32)>, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
    let devx = x0 - 2.0 * x1 + x2;
    let devy = y0 - 2.0 * y1 + y2;
    let devsq = devx * devx + devy * devy;
    if devsq < 0.333 {
        out.push((x0, y0, x2, y2));
        return;
    }
    let n = 1 + (3.0 * devsq).sqrt().sqrt().floor() as usize;
    let (mut px, mut py) = (x0, y0);
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let x = mt * mt * x0 + 2.0 * mt * t * x1 + t * t * x2;
        let y = mt * mt * y0 + 2.0 * mt * t * y1 + t * t * y2;
        out.push((px, py, x, y));
        px = x;
        py = y;
    }
}

/// Signed area accumulation (the font-rs algorithm).
fn draw_line(a: &mut [f32], w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
    if (y0 - y1).abs() < f32::EPSILON {
        return;
    }
    let (dir, (x0, y0), (x1, y1)) = if y0 < y1 { (1.0, (x0, y0), (x1, y1)) } else { (-1.0, (x1, y1), (x0, y0)) };
    let dxdy = (x1 - x0) / (y1 - y0);
    let mut x = x0;
    let y0i = y0.max(0.0) as usize;
    if y0 < 0.0 {
        x -= y0 * dxdy;
    }
    let yend = (y1.ceil() as usize).min(h);
    for y in y0i..yend {
        let linestart = y * w;
        let dy = ((y + 1) as f32).min(y1) - (y as f32).max(y0);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let xa_floor = xa.floor();
        let xa_i = xa_floor as isize;
        let xb_ceil = xb.ceil();
        let xb_i = xb_ceil as isize;
        let at = |i: isize| -> usize { (linestart as isize + i).max(0) as usize };
        if xb_i <= xa_i + 1 {
            let xmf = 0.5 * (x + xnext) - xa_floor;
            let i = at(xa_i);
            if i + 1 < a.len() {
                a[i] += d - d * xmf;
                a[i + 1] += d * xmf;
            }
        } else {
            let s = 1.0 / (xb - xa);
            let xa_f = xa - xa_floor;
            let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
            let xb_f = xb - xb_ceil + 1.0;
            let am = 0.5 * s * xb_f * xb_f;
            let i0 = at(xa_i);
            if i0 < a.len() {
                a[i0] += d * a0;
            }
            if xb_i == xa_i + 2 {
                if i0 + 1 < a.len() {
                    a[i0 + 1] += d * (1.0 - a0 - am);
                }
            } else {
                let a1 = s * (1.5 - xa_f);
                if i0 + 1 < a.len() {
                    a[i0 + 1] += d * (a1 - a0);
                }
                for xi in (xa_i + 2)..(xb_i - 1) {
                    let i = at(xi);
                    if i < a.len() {
                        a[i] += d * s;
                    }
                }
                let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                let i = at(xb_i - 1);
                if i < a.len() {
                    a[i] += d * (1.0 - a2 - am);
                }
            }
            let i = at(xb_i);
            if i < a.len() {
                a[i] += d * am;
            }
        }
        x = xnext;
    }
}

/// Font metrics at one pixel size, rounded like SDL_ttf.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub ascent: i32,
    #[allow(dead_code)]
    pub descent: i32,
    pub height: i32,
    pub line_skip: i32,
}

impl Font {
    pub fn metrics(&self, pixel_size: f32) -> Metrics {
        let s = self.scale_for(pixel_size);
        let ascent = (self.ascender * s).ceil() as i32;
        let descent = (self.descender * s).ceil() as i32;
        let line_skip = ((self.ascender - self.descender + self.line_gap) * s).ceil() as i32;
        Metrics { ascent, descent, height: ascent - descent, line_skip }
    }

    /// Width of a line of text in pixels.
    pub fn text_width(&self, pixel_size: f32, text: &str) -> i32 {
        let s = self.scale_for(pixel_size);
        text.chars().map(|c| (self.advance_units(self.glyph_index(c)) * s).round() as i32).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_font() {
        let f = Font::load("data/TSCu_Comic.ttf").unwrap();
        let g = f.glyph_index('A');
        assert!(g > 0);
        let gl = f.rasterize(g, f.scale_for(40.0));
        assert!(gl.w > 10 && gl.h > 10);
        // an 'O' has a hole: the middle must be empty, the ring full
        let o = f.rasterize(f.glyph_index('O'), f.scale_for(60.0));
        let mid = o.coverage[(o.h / 2) * o.w + o.w / 2];
        let ring = o.coverage[(o.h / 2) * o.w..(o.h / 2 + 1) * o.w].iter().copied().max().unwrap();
        assert!(mid < 30 && ring > 200, "mid {} ring {}", mid, ring);
        let m = f.metrics(20.0);
        assert!(m.height >= 18 && m.height <= 30, "{:?}", m);
        assert!(f.text_width(20.0, "Single Play") > 60);
    }
}

//! PNG decoder (all colour types and bit depths, Adam7) and a minimal
//! encoder (RGBA, stored zlib blocks) for screenshots.

use crate::image::Image;
use crate::inflate;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG file".into());
    }
    let mut pos = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Option<Vec<u8>> = None;
    while pos + 8 <= data.len() {
        let len = be32(&data[pos..]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body_start = pos + 8;
        let body_end = body_start + len;
        if body_end + 4 > data.len() {
            return Err("truncated PNG chunk".into());
        }
        let body = &data[body_start..body_end];
        match kind {
            b"IHDR" => {
                if body.len() < 13 {
                    return Err("bad IHDR".into());
                }
                w = be32(body) as usize;
                h = be32(&body[4..]) as usize;
                depth = body[8];
                ctype = body[9];
                interlace = body[12];
            }
            b"PLTE" => {
                palette = body.chunks(3).filter(|c| c.len() == 3).map(|c| [c[0], c[1], c[2], 255]).collect();
            }
            b"tRNS" => trns = Some(body.to_vec()),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos = body_end + 4;
    }
    if w == 0 || h == 0 || w > 1 << 16 || h > 1 << 16 {
        return Err("bad PNG size".into());
    }
    if let Some(t) = &trns {
        if ctype == 3 {
            for (i, a) in t.iter().enumerate() {
                if i < palette.len() {
                    palette[i][3] = *a;
                }
            }
        }
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(format!("unsupported PNG colour type {}", ctype)),
    };
    if ![1, 2, 4, 8, 16].contains(&depth) {
        return Err(format!("unsupported PNG bit depth {}", depth));
    }
    let bits_pp = channels * depth as usize;
    let bpp = bits_pp.div_ceil(8).max(1); // filter byte distance
    let raw = inflate::zlib_decompress(&idat, h * (1 + (w * bits_pp).div_ceil(8)))?;

    let mut img = Image::new(w, h);
    // transparent colour for grey / RGB
    let key: Option<[u16; 3]> = match (&trns, ctype) {
        (Some(t), 0) if t.len() >= 2 => {
            let g = u16::from_be_bytes([t[0], t[1]]);
            Some([g, g, g])
        }
        (Some(t), 2) if t.len() >= 6 => Some([
            u16::from_be_bytes([t[0], t[1]]),
            u16::from_be_bytes([t[2], t[3]]),
            u16::from_be_bytes([t[4], t[5]]),
        ]),
        _ => None,
    };

    let passes: Vec<(usize, usize, usize, usize)> = if interlace == 1 {
        vec![(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    } else {
        vec![(0, 0, 1, 1)]
    };
    let mut off = 0usize;
    for (x0, y0, dx, dy) in passes {
        if x0 >= w || y0 >= h {
            continue;
        }
        let pw = (w - x0).div_ceil(dx);
        let ph = (h - y0).div_ceil(dy);
        let stride = (pw * bits_pp).div_ceil(8);
        let mut prev = vec![0u8; stride];
        let mut cur = vec![0u8; stride];
        for row in 0..ph {
            if off + 1 + stride > raw.len() {
                return Err("PNG data too short".into());
            }
            let ft = raw[off];
            cur.copy_from_slice(&raw[off + 1..off + 1 + stride]);
            off += 1 + stride;
            unfilter(ft, &mut cur, &prev, bpp)?;
            for col in 0..pw {
                let s = |i: usize| -> u16 {
                    // sample i of this pixel
                    let bit = (col * channels + i) * depth as usize;
                    match depth {
                        8 => cur[bit / 8] as u16,
                        16 => u16::from_be_bytes([cur[bit / 8], cur[bit / 8 + 1]]),
                        d => {
                            let byte = cur[bit / 8];
                            let shift = 8 - d as usize - (bit % 8);
                            ((byte >> shift) & ((1u8 << d) - 1)) as u16
                        }
                    }
                };
                let to8 = |v: u16| -> u8 {
                    match depth {
                        16 => (v >> 8) as u8,
                        8 => v as u8,
                        d => (v as u32 * 255 / ((1u32 << d) - 1)) as u8,
                    }
                };
                let px = match ctype {
                    0 => {
                        let g = s(0);
                        let a = if key == Some([g, g, g]) { 0 } else { 255 };
                        let g8 = to8(g);
                        [g8, g8, g8, a]
                    }
                    2 => {
                        let (r, g, b) = (s(0), s(1), s(2));
                        let a = if key == Some([r, g, b]) { 0 } else { 255 };
                        [to8(r), to8(g), to8(b), a]
                    }
                    3 => {
                        let i = s(0) as usize;
                        *palette.get(i).unwrap_or(&[0, 0, 0, 255])
                    }
                    4 => {
                        let g = to8(s(0));
                        [g, g, g, to8(s(1))]
                    }
                    _ => [to8(s(0)), to8(s(1)), to8(s(2)), to8(s(3))],
                };
                img.set(x0 + col * dx, y0 + row * dy, px);
            }
            std::mem::swap(&mut prev, &mut cur);
        }
    }
    Ok(img)
}

fn unfilter(ft: u8, cur: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), String> {
    let n = cur.len();
    match ft {
        0 => {}
        1 => {
            for i in bpp..n {
                cur[i] = cur[i].wrapping_add(cur[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                cur[i] = cur[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..n {
                let left = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                cur[i] = cur[i].wrapping_add(((left + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..n {
                let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
                let b = prev[i] as i16;
                let c = if i >= bpp { prev[i - bpp] as i16 } else { 0 };
                let p = a + b - c;
                let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                let pred = if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                };
                cur[i] = cur[i].wrapping_add(pred as u8);
            }
        }
        _ => return Err(format!("bad PNG filter type {}", ft)),
    }
    Ok(())
}

fn chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = inflate::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encode an RGBA image (uncompressed deflate: big, but simple).
pub fn encode(img: &Image) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(img.w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(img.h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut raw = Vec::with_capacity(img.h * (img.w * 4 + 1));
    for row in 0..img.h {
        raw.push(0);
        raw.extend_from_slice(&img.px[row * img.w * 4..(row + 1) * img.w * 4]);
    }
    chunk(&mut out, b"IDAT", &inflate::zlib_store(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(path: &str, reference: &str) {
        let img = Image::load(path).unwrap();
        let want = std::fs::read(reference).unwrap();
        assert_eq!(img.px.len(), want.len(), "{}", path);
        assert!(img.px == want, "{} decodes differently from PIL", path);
    }

    #[test]
    fn rgba() {
        check("tests/data/walk.png", "tests/data/walk.rgba");
    }

    #[test]
    fn rgb() {
        check("tests/data/theme_bg.png", "tests/data/theme_bg.rgba");
    }

    #[test]
    fn palette() {
        check("tests/data/icon.png", "tests/data/icon.rgba");
    }

    #[test]
    fn every_game_png_decodes() {
        fn walk(dir: &std::path::Path, n: &mut usize) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, n);
                } else if p.extension().map(|x| x == "png").unwrap_or(false) {
                    Image::load(p.to_str().unwrap()).unwrap();
                    *n += 1;
                }
            }
        }
        let mut n = 0;
        walk(std::path::Path::new("data"), &mut n);
        walk(std::path::Path::new("plugins"), &mut n);
        assert!(n > 50);
    }

    #[test]
    fn roundtrip() {
        let mut img = Image::new(7, 5);
        for (i, p) in img.px.iter_mut().enumerate() {
            *p = (i * 37 % 256) as u8;
        }
        let back = decode(&encode(&img)).unwrap();
        assert!(back.px == img.px);
    }
}

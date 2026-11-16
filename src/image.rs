//! In-memory RGBA images, loaded from PNG or JPEG.

use crate::{jpeg, png};

#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    /// RGBA, row 0 at the top.
    pub px: Vec<u8>,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Image {
        Image { w, h, px: vec![0; w * h * 4] }
    }

    pub fn get(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.w + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    pub fn set(&mut self, x: usize, y: usize, c: [u8; 4]) {
        let i = (y * self.w + x) * 4;
        self.px[i..i + 4].copy_from_slice(&c);
    }

    /// Decode PNG or JPEG data (detected from the first bytes).
    pub fn decode(data: &[u8]) -> Result<Image, String> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            png::decode(data)
        } else if data.starts_with(&[0xff, 0xd8]) {
            jpeg::decode(data)
        } else {
            Err("unknown image format (only PNG and JPEG are supported)".into())
        }
    }

    pub fn load(path: &str) -> Result<Image, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
        Image::decode(&data).map_err(|e| format!("{}: {}", path, e))
    }

    /// A copy of a rectangle of the image (the part inside the image).
    #[cfg(test)]
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        let mut out = Image::new(w, h);
        for row in 0..h.min(self.h.saturating_sub(y)) {
            let n = w.min(self.w.saturating_sub(x)) * 4;
            let s = ((y + row) * self.w + x) * 4;
            out.px[row * w * 4..row * w * 4 + n].copy_from_slice(&self.px[s..s + n]);
        }
        out
    }

    /// The old OpenGL code padded textures to a power of two with the image
    /// in the bottom left corner; sizes (and so draw positions) depend on it.
    pub fn pad_pow2(self) -> Image {
        let tw = self.w.next_power_of_two();
        let th = self.h.next_power_of_two();
        if tw == self.w && th == self.h {
            return self;
        }
        let mut out = Image::new(tw, th);
        let oy = th - self.h;
        for row in 0..self.h {
            let s = row * self.w * 4;
            let d = ((row + oy) * tw) * 4;
            out.px[d..d + self.w * 4].copy_from_slice(&self.px[s..s + self.w * 4]);
        }
        out
    }
}

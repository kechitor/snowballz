//! Full screen backgrounds (menu, loading, winner, loser).
//!
//! data/bg_*.jpg are 3440x1440: the original 640x480 picture is the
//! 1920x1440 middle part, so "design coordinates" (the 640x480 space the
//! menu is laid out in) map onto the screen with one scale factor and the
//! picture fills any screen between 4:3 and 21:9 without black bars.

use crate::gfx::{self, Tex};
use crate::sdl::Rect;
use std::cell::RefCell;

pub const BG_W: f64 = 3440.0;
pub const BG_H: f64 = 1440.0;
pub const DESIGN_W: f64 = 640.0;
pub const DESIGN_H: f64 = 480.0;

thread_local! {
    // keep only the last two (each is ~20 MB of texture memory)
    static CACHE: RefCell<Vec<(String, Tex)>> = const { RefCell::new(Vec::new()) };
}

fn texture(name: &str) -> Option<Tex> {
    if let Some(t) = CACHE.with(|c| c.borrow().iter().find(|(n, _)| n == name).map(|(_, t)| *t)) {
        return Some(t);
    }
    let path = format!("data/bg_{}.jpg", name);
    let img = match crate::image::Image::load(&path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{}", e);
            return None;
        }
    };
    let t = gfx::texture_from_image(&img).ok()?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 2 {
            let (_, old) = c.remove(0);
            gfx::free_texture(&old);
        }
        c.push((name.to_string(), t));
    });
    Some(t)
}

/// Load a background ahead of time (decoding takes a moment).
pub fn preload(name: &str) {
    texture(name);
}

/// (scale, off_x, off_y) mapping 640x480 design coordinates to screen
/// pixels: the design area is centred and as large as fits.
pub fn design_view(sw: i32, sh: i32) -> (f64, f64, f64) {
    let (sw, sh) = (sw as f64, sh as f64);
    let scale = (sw / DESIGN_W).min(sh / DESIGN_H);
    (scale, (sw - DESIGN_W * scale) / 2.0, (sh - DESIGN_H * scale) / 2.0)
}

/// Draw background `name` filling the screen (centred, cropped).
pub fn draw(name: &str, alpha: f32) {
    let (sw, sh) = gfx::screen_size();
    let Some(t) = texture(name) else {
        gfx::fill_screen_rect(Rect::new(0, 0, sw, sh), (20, 20, 40, 255));
        return;
    };
    let s = (sh as f64 / BG_H).max(sw as f64 / BG_W);
    let w = (BG_W * s).round() as i32;
    let h = (BG_H * s).round() as i32;
    gfx::blit(&t, None, Rect::new((sw - w) / 2, (sh - h) / 2, w, h), alpha.clamp(0.0, 1.0));
}

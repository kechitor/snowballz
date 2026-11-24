//! The in-game HUD's look: rounded "frost glass" panels (dark midnight with
//! a thin icy edge), drawn with the white pieces in data/hud/
//! (tools/sprites/make_hud.py) coloured as needed.  Positions here are
//! screen pixels from the top left (y down).

use crate::gfx::{self, Font, Tex};
use crate::sdl::Rect;

/// a panel's glass, and its edge
pub const GLASS: [f32; 4] = [0.047, 0.086, 0.172, 0.8];
pub const EDGE: [f32; 4] = [0.63, 0.8, 1.0, 0.32];
/// a button or a box on a panel
pub const TILE: [f32; 4] = [1.0, 1.0, 1.0, 0.07];
pub const TEXT: [f32; 4] = [0.92, 0.95, 1.0, 1.0];
pub const DIM: [f32; 4] = [0.66, 0.75, 0.85, 1.0];
/// things to click, and crystal / warnings
pub const ICE: [f32; 4] = [0.5, 0.82, 1.0, 1.0];
pub const GOLD: [f32; 4] = [1.0, 0.79, 0.3, 1.0];

pub const RADIUS: f64 = 14.0;

/// The icons in data/hud/icons.png.
#[derive(Clone, Copy)]
pub enum Icon {
    Fish = 0,
    Crystal = 1,
    Penguin = 2,
    Warning = 3,
}

fn piece(name: &str, w: usize) -> Tex {
    gfx::texture_sized(&format!("data/hud/{}.png", name), w)
}

fn rect(x: f64, top: f64, w: f64, h: f64) -> Rect {
    let (l, t) = (x.round() as i32, top.round() as i32);
    Rect::new(l, t, (x + w).round() as i32 - l, (top + h).round() as i32 - t)
}

/// A rounded box.
pub fn fill(x: f64, top: f64, w: f64, h: f64, radius: f64, color: [f32; 4]) {
    gfx::slice9(&piece("round", 32), 12, rect(x, top, w, h), radius.round() as i32, color);
}

/// A rounded box's 1 pixel outline.
pub fn outline(x: f64, top: f64, w: f64, h: f64, radius: f64, color: [f32; 4]) {
    gfx::slice9(&piece("edge", 32), 12, rect(x, top, w, h), radius.round() as i32, color);
}

/// A soft shadow under a box.
pub fn shadow(x: f64, top: f64, w: f64, h: f64, alpha: f32) {
    gfx::slice9(&piece("shadow", 64), 28, rect(x - 16.0, top - 12.0, w + 32.0, h + 32.0), 28, [1.0, 1.0, 1.0, alpha]);
}

/// A glass panel: shadow, glass, edge.
pub fn panel(x: f64, top: f64, w: f64, h: f64) {
    tinted_panel(x, top, w, h, RADIUS, GLASS, EDGE);
}

pub fn tinted_panel(x: f64, top: f64, w: f64, h: f64, radius: f64, glass: [f32; 4], edge: [f32; 4]) {
    shadow(x, top, w, h, 0.28 * glass[3]);
    fill(x, top, w, h, radius, glass);
    outline(x, top, w, h, radius, edge);
}

/// A button or box on a panel; `lit`: chosen / active.
pub fn tile(x: f64, top: f64, w: f64, h: f64, lit: bool) {
    if lit {
        fill(x, top, w, h, 10.0, [0.18, 0.48, 0.88, 0.45]);
        outline(x, top, w, h, 10.0, [0.5, 0.82, 1.0, 1.0]);
        outline(x + 1.0, top + 1.0, w - 2.0, h - 2.0, 9.0, [0.5, 0.82, 1.0, 0.6]);
    } else {
        fill(x, top, w, h, 10.0, TILE);
        outline(x, top, w, h, 10.0, [0.63, 0.8, 1.0, 0.28]);
    }
}

/// A bar `frac` full (rounded ends).
pub fn bar(x: f64, top: f64, w: f64, h: f64, frac: f64, color: [f32; 4]) {
    fill(x, top, w, h, h / 2.0, [1.0, 1.0, 1.0, 0.13 * color[3]]);
    let f = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
    if f > 0.0 {
        fill(x, top, f, h, h / 2.0, color);
    }
}

pub fn icon(i: Icon, x: f64, top: f64, size: f64, color: [f32; 4]) {
    let t = piece("icons", 96).sub(i as i32 * 24, 0, 24, 24);
    let sh = gfx::screen_size().1 as f64;
    t.draw(x.round(), (sh - top - size).round(), color, size / 24.0);
}

/// Text with its top left at x, top.
pub fn text(font: Font, x: f64, top: f64, s: &str, color: [f32; 4]) {
    let (_, th) = font.size_of(s);
    let sh = gfx::screen_size().1 as f64;
    font.render(x.round(), (sh - top - th as f64).round(), s, color);
}

pub fn text_w(font: Font, s: &str) -> f64 {
    font.size_of(s).0 as f64
}

pub fn text_h(font: Font, s: &str) -> f64 {
    font.size_of(s).1 as f64
}

/// A key on the keyboard (like "P"), its top left at x, top; its width.
pub fn keycap(font: Font, x: f64, top: f64, key: &str) -> f64 {
    let (tw, th) = (text_w(font, key), text_h(font, key));
    let (w, h) = ((tw + 12.0).max(22.0), 22.0);
    fill(x, top + 2.0, w, h, 6.0, [0.0, 0.0, 0.0, 0.25]);
    fill(x, top, w, h, 6.0, [0.25, 0.32, 0.45, 1.0]);
    outline(x, top, w, h, 6.0, [1.0, 1.0, 1.0, 0.3]);
    text(font, x + (w - tw) / 2.0, top + (h - th) / 2.0, key, TEXT);
    w
}

/// Is p in the box?
pub fn hit(p: (i32, i32), x: f64, top: f64, w: f64, h: f64) -> bool {
    let (px, py) = (p.0 as f64, p.1 as f64);
    px >= x && px < x + w && py >= top && py < top + h
}

//! The HUD: what you have (top left), the command bar (what is selected
//! and what it can do, bottom middle; selectctrl.py), the minimap
//! (minimap.py), squads, the crystal race, alliances and alerts.  Drawn as
//! frost glass panels (glass.rs).

use crate::gfx;
use crate::glass::{self, Icon};
use crate::world::*;

/// The gap between the HUD and the screen's edges.
pub const MARGIN: f64 = 12.0;

/// Selected penguins per kind: (workers, snowballers).
pub fn selected_counts(w: &World) -> (usize, usize) {
    let mut c = (0, 0);
    for &u in &w.selected {
        match w.units[u].kind {
            UnitKind::Worker => c.0 += 1,
            UnitKind::Snowballer => c.1 += 1,
        }
    }
    c
}

// --- what you have ------------------------------------------------------

/// The strip's height (the alert goes under it).
const STRIP_H: f64 = 44.0;

/// Top left: my fish, crystal and penguins.
pub fn draw_resources(w: &World) {
    let Some(me) = w.me else { return };
    if w.chat.chatting {
        return;
    }
    let mine = |kind: ResKind| -> i32 { (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(me)).map(|b| w.num_storage(b, kind)).sum() };
    let penguins = (0..w.units.len()).filter(|&u| w.igloo_player(u) == Some(me)).count();
    let items = [
        (Icon::Fish, mine(ResKind::Fish).to_string(), "fish", glass::ICE),
        (Icon::Crystal, mine(ResKind::Crystal).to_string(), "crystal", glass::GOLD),
        (Icon::Penguin, penguins.to_string(), "penguins", glass::TEXT),
    ];
    let (big, small) = (w.chat_font, w.stat_font);
    let widths: Vec<f64> = items.iter().map(|(_, n, l, _)| 12.0 + 20.0 + 8.0 + glass::text_w(big, n) + 6.0 + glass::text_w(small, l) + 12.0).collect();
    let width = widths.iter().sum::<f64>() + 6.0 * (items.len() as f64 + 1.0);
    let (x0, top) = (MARGIN, MARGIN);
    glass::panel(x0, top, width, STRIP_H);
    let mut x = x0 + 6.0;
    for ((icon, n, label, c), iw) in items.iter().zip(widths) {
        let (t, h) = (top + 6.0, STRIP_H - 12.0);
        glass::fill(x, t, iw, h, 10.0, glass::TILE);
        if let Icon::Penguin = icon {
            // (a little penguin in my colour reads better than an icon)
            let pc = w.me_color();
            let (s, sh) = (0.9, w.view.h as f64);
            let (px, py) = (x + 12.0 + 10.0 - 16.0 * s, sh - (t + h / 2.0 - 16.0 * s) - 32.0 * s);
            w.tex.penguins["walk"].get(10, (0, 1)).draw(px, py, [1.0; 4], s);
            w.tex.penguins["workerwalk"].get(10, (0, 1)).draw(px, py, [pc[0], pc[1], pc[2], 1.0], s);
        } else {
            glass::icon(*icon, x + 12.0, t + (h - 20.0) / 2.0, 20.0, *c);
        }
        let nx = x + 12.0 + 20.0 + 8.0;
        glass::text(big, nx, t + (h - glass::text_h(big, n)) / 2.0, n, glass::TEXT);
        let lx = nx + glass::text_w(big, n) + 6.0;
        glass::text(small, lx, t + (h - glass::text_h(small, label)) / 2.0 + 1.0, label, glass::DIM);
        x += iw + 6.0;
    }
}

// --- the command bar ----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PanelClick {
    Snowballers,
    Workers,
    Disband,
    SnowWall,
    IceWall,
    SwitchClass,
    Nothing,
}

const PAD: f64 = 10.0;
const PORTRAIT_W: f64 = 74.0;
const TILE_H: f64 = 86.0;
const INFO_W: f64 = 150.0;
const ACTION_W: f64 = 84.0;

/// Where the bar is (x, top, width, height; y down) and its parts.
struct Bar {
    x: f64,
    top: f64,
    w: f64,
    h: f64,
    portraits: Vec<(f64, UnitKind, usize)>,
    info_x: f64,
    divider_x: f64,
    actions: Vec<(f64, f64, PanelClick)>,
}

fn bar(w: &World) -> Option<Bar> {
    if w.selected.is_empty() {
        return None;
    }
    let (workers, snowballers) = selected_counts(w);
    let mut x = PAD;
    let mut portraits = Vec::new();
    for (kind, n) in [(UnitKind::Worker, workers), (UnitKind::Snowballer, snowballers)] {
        if n > 0 {
            portraits.push((x, kind, n));
            x += PORTRAIT_W + 8.0;
        }
    }
    let info_x = x + 6.0;
    let divider_x = info_x + INFO_W + 14.0;
    x = divider_x + 15.0;
    let mut actions = vec![(x, ACTION_W, PanelClick::Disband)];
    x += ACTION_W + 8.0;
    if workers > 0 {
        for a in [PanelClick::SnowWall, PanelClick::IceWall] {
            actions.push((x, ACTION_W, a));
            x += ACTION_W + 8.0;
        }
    }
    actions.push((x, 100.0, PanelClick::SwitchClass));
    x += 100.0 + PAD;
    let (vw, vh) = (w.view.w as f64, w.view.h as f64);
    let h = TILE_H + 2.0 * PAD;
    // in the middle, but not over the minimap
    let right = if w.draw_minimap { minimap_rect(w).0 - 8.0 - MARGIN } else { vw - MARGIN };
    let left = ((vw - x) / 2.0).min(right - x).max(MARGIN).round();
    Some(Bar { x: left, top: vh - MARGIN - h, w: x, h, portraits, info_x, divider_x, actions })
}

fn action_text(a: PanelClick) -> (&'static str, &'static str, String) {
    match a {
        PanelClick::Disband => ("O", "Stop", String::new()),
        PanelClick::SnowWall => ("B", "Snow wall", "free".into()),
        PanelClick::IceWall => ("U", "Ice wall", format!("{} crystal", crate::wall::ICE_CRYSTAL)),
        PanelClick::SwitchClass => ("X", "Change class", format!("{} fish each", crate::unit::SWITCH_FISH)),
        _ => ("", "", String::new()),
    }
}

/// Draw the bar (ui mode).
pub fn draw_panel(w: &World) {
    let Some(b) = bar(w) else { return };
    let mouse = gfx::mouse_pos();
    glass::panel(b.x, b.top, b.w, b.h);
    let top = b.top + PAD;
    let pc = w.me_color();
    let sh = w.view.h as f64;
    let only_one = b.portraits.len() == 1;
    for &(px, kind, n) in &b.portraits {
        let x = b.x + px;
        glass::fill(x, top, PORTRAIT_W, TILE_H, 10.0, [0.84, 0.87, 0.92, 1.0]);
        let hover = glass::hit(mouse, x, top, PORTRAIT_W, TILE_H);
        let edge = if only_one || hover { [0.5, 0.82, 1.0, 1.0] } else { [0.63, 0.8, 1.0, 0.45] };
        glass::outline(x, top, PORTRAIT_W, TILE_H, 10.0, edge);
        if only_one {
            glass::outline(x + 1.0, top + 1.0, PORTRAIT_W - 2.0, TILE_H - 2.0, 9.0, edge);
        }
        // the penguin, facing us, in my colour
        let outfit = if kind == UnitKind::Worker { "workerwalk" } else { "snowballerwalk" };
        let s = 2.1;
        let (dx, dtop) = (x + (PORTRAIT_W - 32.0 * s) / 2.0, top + 4.0);
        let y = sh - dtop - 32.0 * s;
        w.tex.penguins["walk"].get(10, (0, 1)).draw(dx, y, [1.0; 4], s);
        w.tex.penguins[outfit].get(10, (0, 1)).draw(dx, y, [pc[0], pc[1], pc[2], 1.0], s);
        // how many
        let count = n.to_string();
        let cw = glass::text_w(w.stat_font, &count) + 14.0;
        glass::fill(x + PORTRAIT_W - cw - 4.0, top + TILE_H - 26.0, cw, 22.0, 11.0, [0.047, 0.086, 0.172, 0.88]);
        glass::text(w.stat_font, x + PORTRAIT_W - cw + 3.0, top + TILE_H - 25.0, &count, glass::TEXT);
    }
    // what is selected, and how warm they are
    let (workers, snowballers) = selected_counts(w);
    let plural = |n: usize, one: &str| format!("{} {}{}", n, one, if n == 1 { "" } else { "s" });
    let title = match (workers, snowballers) {
        (n, 0) => plural(n, "worker"),
        (0, n) => plural(n, "snowballer"),
        (a, b) => plural(a + b, "penguin"),
    };
    let ix = b.x + b.info_x;
    glass::text(w.chat_font, ix, top + 10.0, &title, glass::TEXT);
    glass::text(w.stat_font, ix, top + 42.0, "Warmth", glass::DIM);
    let (warm, max) = w.selected.iter().fold((0.0, 0.0), |(a, m), &u| (a + w.units[u].warmth.max(0) as f64, m + w.units[u].max_warmth.max(1) as f64));
    let f = if max > 0.0 { warm / max } else { 0.0 };
    let c = if f > 0.5 { [0.34, 0.84, 0.55, 1.0] } else if f > 0.25 { [1.0, 0.75, 0.3, 1.0] } else { [1.0, 0.4, 0.3, 1.0] };
    glass::bar(ix, top + 66.0, INFO_W, 8.0, f, c);
    gfx::fill_screen_rect(crate::sdl::Rect::new((b.x + b.divider_x) as i32, top as i32, 1, TILE_H as i32), (160, 205, 255, 64));
    // what they can do
    for &(ax, aw, a) in &b.actions {
        let x = b.x + ax;
        let lit = matches!((a, w.wall_mode), (PanelClick::SnowWall, Some(crate::wall::WallMode::Build)) | (PanelClick::IceWall, Some(crate::wall::WallMode::Ice)));
        let hover = glass::hit(mouse, x, top, aw, TILE_H);
        if a == PanelClick::SwitchClass {
            glass::fill(x, top, aw, TILE_H, 10.0, [1.0, 0.79, 0.3, if hover { 0.24 } else { 0.14 }]);
            glass::outline(x, top, aw, TILE_H, 10.0, [1.0, 0.79, 0.3, 0.6]);
        } else {
            glass::tile(x, top, aw, TILE_H, lit);
            if hover && !lit {
                glass::fill(x, top, aw, TILE_H, 10.0, [1.0, 1.0, 1.0, 0.07]);
            }
        }
        let (key, label, sub) = action_text(a);
        let kw = glass::text_w(w.stat_font, key) + 12.0;
        glass::keycap(w.stat_font, x + (aw - kw.max(22.0)) / 2.0, top + 10.0, key);
        let lc = if a == PanelClick::SwitchClass { [1.0, 0.91, 0.68, 1.0] } else { glass::TEXT };
        glass::text(w.stat_font, x + (aw - glass::text_w(w.stat_font, label)) / 2.0, top + 38.0, label, lc);
        if !sub.is_empty() {
            let small = w.stat_font.with_size(13);
            let sc = if a == PanelClick::SwitchClass { [0.91, 0.79, 0.48, 1.0] } else { glass::DIM };
            glass::text(small, x + (aw - glass::text_w(small, &sub)) / 2.0, top + 60.0, &sub, sc);
        }
    }
}

/// Planning a wall: what to do (top middle, under the power-up countdown).
pub fn draw_wall_mode(w: &World) {
    use crate::wall::*;
    let Some(mode) = w.wall_mode else { return };
    let text = match mode {
        WallMode::Build => format!("Snow wall: click to place an {}-tile section (free, {} s of work; more workers build faster)", SECTION_TILES, BUILD_MS / 1000),
        WallMode::Ice => format!("Ice wall: click a section of your snow wall ({} crystal, {} s of work)", ICE_CRYSTAL, UPGRADE_MS / 1000),
    };
    let sub = match mode {
        WallMode::Build => "R / mouse wheel: rotate   Shift: keep placing   Right click your wall: take it down   Right click / Esc: stop",
        WallMode::Ice => "Shift: keep going   Right click / Esc: stop",
    };
    let vw = w.view.w as f64;
    let font = w.stat_font;
    let (tw, sw, th) = (glass::text_w(font, &text), glass::text_w(font, sub), glass::text_h(font, &text));
    let bw = tw.max(sw) + 32.0;
    let bh = th * 2.0 + 26.0;
    let (x, top) = ((vw - bw) / 2.0, MARGIN + 44.0 + 10.0);
    glass::panel(x, top, bw, bh);
    glass::text(font, (vw - tw) / 2.0, top + 10.0, &text, glass::TEXT);
    glass::text(font, (vw - sw) / 2.0, top + 14.0 + th, sub, glass::DIM);
}

/// Is the (screen) position over the bar?
pub fn in_panel(w: &World, pos: (i32, i32)) -> bool {
    bar(w).is_some_and(|b| glass::hit(pos, b.x, b.top, b.w, b.h))
}

pub fn panel_click(w: &World, pos: (i32, i32)) -> PanelClick {
    let Some(b) = bar(w) else { return PanelClick::Nothing };
    let top = b.top + PAD;
    for &(px, kind, _) in &b.portraits {
        if glass::hit(pos, b.x + px, top, PORTRAIT_W, TILE_H) {
            return if kind == UnitKind::Worker { PanelClick::Workers } else { PanelClick::Snowballers };
        }
    }
    for &(ax, aw, a) in &b.actions {
        if glass::hit(pos, b.x + ax, top, aw, TILE_H) {
            return a;
        }
    }
    PanelClick::Nothing
}

/// Keep only the selected penguins of one kind.
pub fn keep_kind(w: &mut World, kind: UnitKind) {
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    for u in sel {
        if w.units[u].kind != kind {
            w.units[u].selected = false;
            w.selected.remove(&u);
        }
    }
}

/// The minimap's place on the screen (left, top, width, height; y down)
/// and its pixels per tile.
fn minimap_rect(w: &World) -> (f64, f64, f64, f64, f64) {
    let (vw, vh) = (w.view.w as f64, w.view.h as f64);
    let (mw, mh) = (w.size.0.max(1) as f64, w.size.1.max(1) as f64);
    let side = (vh * MINIMAP_SHARE).clamp(120.0, 320.0);
    let k = side / mw.max(mh);
    let (bw, bh) = (mw * k, mh * k);
    (vw - MINIMAP_MARGIN - bw, vh - MINIMAP_MARGIN - bh, bw, bh, k)
}

// -- minimap ------------------------------------------------------------------

/// The minimap, in the bottom right corner: the land (snow, ice, trees,
/// fish holes, crystal), igloos and penguins in their players' colours,
/// walls, power-ups, the view, and a blinking alert.  Left click (or drag)
/// moves the view; right click sends the selected penguins there.  Space
/// hides / shows it.
pub struct Minimap {
    land: Option<gfx::Tex>,
}

/// The minimap's size: its longer side, as a share of the screen height ...
const MINIMAP_SHARE: f64 = 0.3;
/// ... and the gap to the screen's edges (its glass frame is 8 wide).
const MINIMAP_MARGIN: f64 = MARGIN + 8.0;

impl Minimap {
    pub fn new(w: &World) -> Minimap {
        let (mw, mh) = (w.size.0.max(1) as usize, w.size.1.max(1) as usize);
        let mut img = crate::image::Image::new(mw, mh);
        let (lo, hi) = w.tiles.iter().fold((f64::MAX, f64::MIN), |(a, b), t| (a.min(t.elevation), b.max(t.elevation)));
        for t in &w.tiles {
            let mut c: [f32; 3] = match t.kind {
                TileKind::Snow => [232.0, 236.0, 244.0],
                TileKind::Ice => [168.0, 196.0, 232.0],
            };
            // (higher ground lighter, lower darker)
            let e = if hi > lo { ((t.elevation - lo) / (hi - lo)) as f32 } else { 0.5 };
            let k = 0.82 + 0.18 * e;
            c = [c[0] * k, c[1] * k, c[2] * k];
            if let Some(r) = t.resource {
                c = match w.resources[r].kind {
                    ResKind::Tree => [52.0, 128.0, 62.0],
                    ResKind::IcePinnacle => [120.0, 150.0, 200.0],
                    ResKind::Fish => [60.0, 100.0, 210.0],
                    ResKind::Crystal => [120.0, 230.0, 250.0],
                };
            }
            let i = ((t.y as usize) * mw + t.x as usize) * 4;
            if i + 3 < img.px.len() {
                img.px[i..i + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
            }
        }
        Minimap { land: gfx::texture_from_image(&img).ok() }
    }

    /// Where it is: (x, top, width, height) in screen pixels (y down), and
    /// pixels per tile.
    fn rect(&self, w: &World) -> (f64, f64, f64, f64, f64) {
        minimap_rect(w)
    }

    /// The tile under a screen position (y down), if on the minimap.
    pub fn tile_at(&self, w: &World, pos: (i32, i32)) -> Option<(f64, f64)> {
        if !w.draw_minimap {
            return None;
        }
        let (x, top, bw, bh, k) = self.rect(w);
        let (px, py) = (pos.0 as f64, pos.1 as f64);
        (px >= x && px < x + bw && py >= top && py < top + bh).then(|| ((px - x) / k, (py - top) / k))
    }

    /// Is the screen position (y down) on it (or its frame)?
    pub fn contains(&self, w: &World, pos: (i32, i32)) -> bool {
        if !w.draw_minimap {
            return false;
        }
        let (x, top, bw, bh, _) = self.rect(w);
        let (px, py) = (pos.0 as f64, pos.1 as f64);
        px >= x - 8.0 && px < x + bw + 8.0 && py >= top - 8.0 && py < top + bh + 8.0
    }

    pub fn draw(&mut self, w: &mut World) {
        if !w.draw_minimap {
            return;
        }
        let sh = w.view.h as f64;
        let (x, top, bw, bh, k) = self.rect(w);
        // (ui mode: y counts from the bottom)
        let yup = |ty: f64, h: f64| sh - ty - h;
        glass::panel(x - 8.0, top - 8.0, bw + 16.0, bh + 16.0);
        if let Some(t) = &self.land {
            gfx::blit(t, None, crate::sdl::Rect::new(x as i32, top as i32, bw.round() as i32, bh.round() as i32), 1.0);
        }
        let dot = |tx: f64, ty: f64, size: f64, c: [f32; 4]| {
            let (px, py) = (x + (tx + 0.5) * k - size / 2.0, top + (ty + 0.5) * k - size / 2.0);
            gfx::fill_rect(px, yup(py, size), size, size, Some(c));
        };
        // walls
        for s in &w.walls {
            if s.alive && s.work > 0 {
                let c = if s.ice { [0.55, 0.8, 1.0, 1.0] } else if s.built() { [0.35, 0.38, 0.45, 1.0] } else { [0.35, 0.38, 0.45, 0.45] };
                dot(s.x as f64, s.y as f64, k.max(1.5), c);
            }
        }
        // power-ups lying about
        for d in w.dmos.iter_mut() {
            if !d.hidden && d.minimapimage.is_some() {
                d.doanimate();
                let a = (0.4 + d.alpha) as f32;
                dot(d.position.0 as f64, d.position.1 as f64, 4.0, [1.0, 0.85, 0.2, a.min(1.0)]);
            }
        }
        // igloos (a dark edge, their player's colour)
        for b in &w.buildings {
            let (cx, cy) = (b.x as f64 + 1.5, b.y as f64 + 1.5);
            dot(cx, cy, 9.0, [0.05, 0.05, 0.1, 1.0]);
            let c = match b.player {
                Some(p) => w.glcolor(Some(p)),
                None => [0.85, 0.85, 0.85],
            };
            dot(cx, cy, 7.0, [c[0], c[1], c[2], 1.0]);
            if b.jeopardy && (crate::sdl::ticks() / 250).is_multiple_of(2) {
                dot(cx, cy, 3.0, [1.0, 1.0, 1.0, 1.0]);
            }
        }
        // penguins
        for ui in 0..w.units.len() {
            let u = &w.units[ui];
            if u.state == State::InBuilding {
                continue;
            }
            let c = w.glcolor(w.igloo_player(ui));
            let a = if u.state == State::Frozen { 0.45 } else { 1.0 };
            dot(u.x as f64, u.y as f64, 2.5, [c[0], c[1], c[2], a]);
        }
        // an alert: a blinking ring
        if let Some(al) = crate::alerts::showing(w) {
            if (crate::sdl::ticks() / 300).is_multiple_of(2) {
                let (ax, ay) = (x + (al.x as f64 + 0.5) * k, top + (al.y as f64 + 0.5) * k);
                let r = 7.0;
                let c = Some([1.0, 0.2, 0.15, 1.0]);
                gfx::fill_rect(ax - r, yup(ay - r, 2.0), 2.0 * r, 2.0, c);
                gfx::fill_rect(ax - r, yup(ay + r - 2.0, 2.0), 2.0 * r, 2.0, c);
                gfx::fill_rect(ax - r, yup(ay - r, 2.0 * r), 2.0, 2.0 * r, c);
                gfx::fill_rect(ax + r - 2.0, yup(ay - r, 2.0 * r), 2.0, 2.0 * r, c);
            }
        }
        // the view
        let (vx, vy) = (x + w.view.x / 32.0 * k, top + w.view.y / 32.0 * k);
        let (vw, vh) = (w.view.scale_w() as f64 / 32.0 * k, w.view.scale_h() as f64 / 32.0 * k);
        let (cx0, cy0) = (vx.max(x), vy.max(top));
        let (cx1, cy1) = ((vx + vw).min(x + bw), (vy + vh).min(top + bh));
        if cx1 > cx0 && cy1 > cy0 {
            let c = Some([1.0, 1.0, 1.0, 0.9]);
            gfx::fill_rect(cx0, yup(cy0, 1.5), cx1 - cx0, 1.5, c);
            gfx::fill_rect(cx0, yup(cy1 - 1.5, 1.5), cx1 - cx0, 1.5, c);
            gfx::fill_rect(cx0, yup(cy0, cy1 - cy0), 1.5, cy1 - cy0, c);
            gfx::fill_rect(cx1 - 1.5, yup(cy0, cy1 - cy0), 1.5, cy1 - cy0, c);
        }
    }
}

// --- squads -----------------------------------------------------------

const SQUAD_W: f64 = 54.0;
const SQUAD_H: f64 = 40.0;
const SQUAD_GAP: f64 = 6.0;
const SQUAD_MARGIN: f64 = MARGIN;

/// The squad buttons along the bottom left: (squad, x, y from the top).
fn squad_buttons(w: &World) -> Vec<(usize, f64, f64)> {
    if w.chat.chatting || w.me.is_none() {
        return Vec::new();
    }
    let top = w.view.h as f64 - SQUAD_MARGIN - SQUAD_H;
    let mut x = SQUAD_MARGIN;
    let mut out = Vec::new();
    for n in 1..=9 {
        if !crate::ctrl::squad_members(w, n).is_empty() {
            out.push((n, x, top));
            x += SQUAD_W + SQUAD_GAP;
        }
    }
    out
}

/// The squad button under this screen position.
pub fn squad_at(w: &World, pos: (i32, i32)) -> Option<usize> {
    let (px, py) = (pos.0 as f64, pos.1 as f64);
    squad_buttons(w).into_iter().find(|&(_, x, y)| px >= x && px < x + SQUAD_W && py >= y && py < y + SQUAD_H).map(|b| b.0)
}

/// One button per squad: its number and how many penguins are in it
/// (lit up while exactly that squad is selected).
pub fn draw_squads(w: &World) {
    for (n, x, top) in squad_buttons(w) {
        let members = crate::ctrl::squad_members(w, n);
        let lit = !w.selected.is_empty() && w.selected.len() == members.len() && members.iter().all(|u| w.selected.contains(u));
        glass::shadow(x, top, SQUAD_W, SQUAD_H, 0.2);
        glass::fill(x, top, SQUAD_W, SQUAD_H, 10.0, glass::GLASS);
        glass::tile(x, top, SQUAD_W, SQUAD_H, lit);
        let num = n.to_string();
        glass::text(w.chat_font, x + 9.0, top + (SQUAD_H - glass::text_h(w.chat_font, &num)) / 2.0, &num, glass::TEXT);
        let count = members.len().to_string();
        let cw = glass::text_w(w.stat_font, &count);
        glass::text(w.stat_font, x + SQUAD_W - cw - 8.0, top + SQUAD_H - glass::text_h(w.stat_font, &count) - 5.0, &count, glass::DIM);
    }
}

// --- crystal race -----------------------------------------------------

/// On "gather N crystal" maps: every player's crystal, top right (allies
/// together: they need the amount times their number).
pub fn draw_crystal_race(w: &World) {
    if w.config.get("victory", "resource_type") != Some("crystal") {
        return;
    }
    let Some(target) = w.config.get_int("victory", "amount") else { return };
    // one row per team (a player alone is a team of one)
    let mut seen: Vec<Pid> = Vec::new();
    let mut rows: Vec<(String, Vec<[f32; 3]>, i32, i32, bool, bool)> = Vec::new();
    for p in w.players.values() {
        if seen.contains(&p.id) {
            continue;
        }
        let team = crate::alliance::team_of(w, p.id);
        seen.extend(team.iter().copied());
        let names: Vec<String> = team.iter().map(|&q| w.player_name(q)).collect();
        let colors: Vec<[f32; 3]> = team.iter().filter_map(|q| w.players.get(q)).map(|q| q.glcolor).collect();
        let crystal: i32 = w.buildings.iter().filter(|b| b.player.is_some_and(|o| team.contains(&o))).map(|b| b.storage[1]).sum();
        let alive = team.iter().any(|&q| crate::alliance::alive(w, q));
        let me = w.me.is_some_and(|m| team.contains(&m));
        rows.push((names.join(" + "), colors, crystal, target as i32 * team.len() as i32, me, alive));
    }
    if rows.is_empty() {
        return;
    }
    // closest to winning first; teams without igloos last
    rows.sort_by(|a, b| b.5.cmp(&a.5).then((b.2 as f64 / b.3 as f64).total_cmp(&(a.2 as f64 / a.3 as f64))).then(a.0.cmp(&b.0)));
    let (big, font) = (w.chat_font, w.stat_font);
    let sw = w.view.w as f64;
    let title = "Crystal race";
    let goal = format!("first to {}", target);
    let lh = glass::text_h(font, "Ag") + 10.0;
    let num_w = rows.iter().map(|r| glass::text_w(font, &format!("{}/{}", r.3, r.3))).fold(0.0, f64::max);
    let name_w = rows.iter().map(|r| glass::text_w(font, &r.0)).fold(0.0, f64::max).min(220.0);
    let swatches = rows.iter().map(|r| r.1.len()).max().unwrap_or(1) as f64;
    let sw_w = swatches * 15.0 - 3.0;
    let bar_w = 64.0;
    let inner = (sw_w + 8.0 + name_w + 10.0 + bar_w + 10.0 + num_w).max(glass::text_w(big, title) + 16.0 + glass::text_w(font, &goal));
    let width = inner + 32.0;
    let head = glass::text_h(big, title) + 14.0;
    let height = 14.0 + head + lh * rows.len() as f64 + 8.0;
    let x0 = sw - width - MARGIN;
    let top = MARGIN;
    glass::panel(x0, top, width, height);
    glass::text(big, x0 + 16.0, top + 12.0, title, glass::GOLD);
    let gw = glass::text_w(font, &goal);
    glass::text(font, x0 + width - 16.0 - gw, top + 12.0 + glass::text_h(big, title) - glass::text_h(font, &goal) - 1.0, &goal, glass::DIM);
    let mut y = top + 14.0 + head;
    for (name, colors, n, goal, me, alive) in rows {
        let a = if alive { 1.0 } else { 0.45 };
        if me {
            glass::fill(x0 + 8.0, y - 1.0, width - 16.0, lh - 2.0, 8.0, [1.0, 1.0, 1.0, 0.1]);
        }
        let mid = y + (lh - 2.0) / 2.0;
        let mut x = x0 + 16.0;
        for (i, c) in colors.iter().enumerate() {
            glass::fill(x + i as f64 * 15.0, mid - 6.0, 12.0, 12.0, 3.0, [c[0], c[1], c[2], a]);
        }
        x += sw_w + 8.0;
        let text_c = [glass::TEXT[0], glass::TEXT[1], glass::TEXT[2], a];
        let th = glass::text_h(font, &name);
        glass::text(font, x, mid - th / 2.0, &name, text_c);
        x += name_w + 10.0;
        let f = n as f64 / goal as f64;
        let c = colors[0];
        glass::bar(x, mid - 3.0, bar_w, 6.0, f, [c[0], c[1], c[2], a]);
        let count = format!("{}/{}", n, goal);
        glass::text(font, x0 + width - 16.0 - glass::text_w(font, &count), mid - th / 2.0, &count, text_c);
        y += lh;
    }
}

// --- alliances --------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AllyAction {
    Offer(Pid),
    Accept(Pid),
    Reject(Pid),
    /// (the host) a computer player's level: the next one
    Level(Pid),
    /// the "P: players & alliances" button
    TogglePanel,
    /// a click on the panel that does nothing (but isn't a map click)
    Nothing,
}

/// A clickable area (screen pixels, y down) and what it does.
#[derive(Clone)]
struct Button {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    label: String,
    action: AllyAction,
    on: bool,
}

impl Button {
    fn hit(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
    fn draw(&self, w: &World) {
        let (fill, edge) = match (self.on, self.action) {
            (false, _) => ([1.0, 1.0, 1.0, 0.05], [0.63, 0.8, 1.0, 0.15]),
            (_, AllyAction::Reject(_)) => ([0.75, 0.2, 0.2, 0.55], [1.0, 0.55, 0.5, 0.8]),
            (_, AllyAction::Accept(_)) => ([0.2, 0.62, 0.35, 0.55], [0.5, 1.0, 0.65, 0.8]),
            _ => ([0.18, 0.48, 0.88, 0.45], [0.5, 0.82, 1.0, 0.9]),
        };
        glass::fill(self.x, self.y, self.w, self.h, 8.0, fill);
        glass::outline(self.x, self.y, self.w, self.h, 8.0, edge);
        let (tw, th) = (glass::text_w(w.stat_font, &self.label), glass::text_h(w.stat_font, &self.label));
        let c = if self.on { glass::TEXT } else { [0.6, 0.65, 0.72, 1.0] };
        glass::text(w.stat_font, self.x + (self.w - tw) / 2.0, self.y + (self.h - th) / 2.0, &self.label, c);
    }
}

const ROW_H: f64 = 34.0;
const PANEL_W: f64 = 580.0;
/// the computer players' level, in the panel's rows
const LEVEL_X: f64 = 190.0;
const LEVEL_W: f64 = 92.0;
const PANEL_X: f64 = MARGIN;
const PANEL_TOP: f64 = MARGIN + STRIP_H + 8.0;
const POPUP_TOP: f64 = MARGIN + 44.0 + 10.0;

/// The other players, in the alliances panel.
fn panel_players(w: &World) -> Vec<Pid> {
    w.players.keys().copied().filter(|&p| Some(p) != w.me).collect()
}

fn panel_height(w: &World) -> f64 {
    ROW_H * (panel_players(w).len() as f64 + 1.5) + 8.0
}

/// The oldest offer to me.
fn offer_to_me(w: &World) -> Option<(Pid, u32)> {
    let me = w.me?;
    w.offers.iter().filter(|o| o.1 == me).min_by_key(|o| o.2).map(|o| (o.0, o.2))
}

fn popup_rect(w: &World) -> Option<(f64, f64, f64, f64, String)> {
    let (from, _) = offer_to_me(w)?;
    let text = format!("{} offers you an alliance ({} fish each)", w.player_name(from), crate::alliance::ALLIANCE_FISH);
    let (tw, _) = w.chat_font.size_of(&text);
    let width = tw as f64 + 24.0 + 2.0 * 96.0 + 20.0;
    let x = ((w.view.w as f64 - width) / 2.0).round();
    Some((x, POPUP_TOP, width, 40.0, text))
}

fn ally_buttons(w: &World) -> Vec<Button> {
    let mut out = Vec::new();
    if let (Some((from, _)), Some((x, y, width, h, _))) = (offer_to_me(w), popup_rect(w)) {
        let bx = x + width - 2.0 * 96.0 - 10.0;
        out.push(Button { x: bx, y: y + 6.0, w: 90.0, h: h - 12.0, label: "Accept (Y)".into(), action: AllyAction::Accept(from), on: true });
        out.push(Button { x: bx + 96.0, y: y + 6.0, w: 90.0, h: h - 12.0, label: "Reject (N)".into(), action: AllyAction::Reject(from), on: true });
    }
    if !w.diplomacy_open {
        return out;
    }
    let Some(me) = w.me else { return out };
    let now = w.now();
    let rich = crate::alliance::fish_of(w, me) >= crate::alliance::ALLIANCE_FISH;
    let mut y = PANEL_TOP + ROW_H * 1.5;
    for p in panel_players(w) {
        let bx = PANEL_X + PANEL_W - 190.0;
        let by = y + 4.0;
        // a computer player's level: the host (or single player) sets it
        if w.is_server() && w.players.get(&p).is_some_and(|pl| pl.kind == PlayerKind::Ai) {
            let l = w.players[&p].ai_level;
            out.push(Button { x: PANEL_X + LEVEL_X, y: by, w: LEVEL_W, h: ROW_H - 8.0, label: format!("AI: {}", l.label()), action: AllyAction::Level(p), on: true });
        }
        if !crate::alliance::alive(w, p) || crate::alliance::allied(w, Some(me), Some(p)) {
            // (status text only)
        } else if w.offers.iter().any(|o| o.0 == p && o.1 == me) {
            out.push(Button { x: bx, y: by, w: 88.0, h: ROW_H - 8.0, label: "Accept".into(), action: AllyAction::Accept(p), on: true });
            out.push(Button { x: bx + 94.0, y: by, w: 88.0, h: ROW_H - 8.0, label: "Reject".into(), action: AllyAction::Reject(p), on: true });
        } else if !w.offers.iter().any(|o| o.0 == me && o.1 == p) {
            out.push(Button { x: bx, y: by, w: 182.0, h: ROW_H - 8.0, label: "Offer alliance".into(), action: AllyAction::Offer(p), on: rich });
        }
        let _ = now;
        y += ROW_H;
    }
    out
}

/// The "P: players & alliances" button (x, top, width, height) and its
/// text, above the squad buttons.
fn hint_rect(w: &World) -> Option<(f64, f64, f64, f64, &'static str)> {
    if w.chat.chatting || w.me.is_none() {
        return None;
    }
    let text = if w.diplomacy_open { "Close players and alliances" } else { "Players and alliances" };
    let h = 36.0;
    let width = 8.0 + 22.0 + 8.0 + glass::text_w(w.stat_font, text) + 12.0;
    let above = if squad_buttons(w).is_empty() { 0.0 } else { SQUAD_H + 8.0 };
    Some((MARGIN, w.view.h as f64 - MARGIN - above - h, width, h, text))
}

/// What a click at this screen position does (None: not on alliance UI).
pub fn ally_click(w: &World, pos: (i32, i32)) -> Option<AllyAction> {
    if let Some((x, y, bw, bh, _)) = hint_rect(w) {
        if glass::hit(pos, x, y, bw, bh) {
            return Some(AllyAction::TogglePanel);
        }
    }
    let p = (pos.0 as f64, pos.1 as f64);
    if let Some(b) = ally_buttons(w).into_iter().find(|b| b.hit(p)) {
        return Some(if b.on { b.action } else { AllyAction::Nothing });
    }
    if let Some((x, y, width, h, _)) = popup_rect(w) {
        if p.0 >= x && p.0 < x + width && p.1 >= y && p.1 < y + h {
            return Some(AllyAction::Nothing);
        }
    }
    if w.diplomacy_open && p.0 >= PANEL_X && p.0 < PANEL_X + PANEL_W && p.1 >= PANEL_TOP && p.1 < PANEL_TOP + panel_height(w) {
        return Some(AllyAction::Nothing);
    }
    None
}

/// The offer pop-up, the alliances panel (P) and the hint for it.
pub fn draw_alliances(w: &World) {
    let Some(me) = w.me else { return };
    let font = w.chat_font;
    let now = w.now();
    if let Some((x, y, bw, bh, text)) = hint_rect(w) {
        glass::panel(x, y, bw, bh);
        if glass::hit(gfx::mouse_pos(), x, y, bw, bh) {
            glass::fill(x, y, bw, bh, glass::RADIUS, [1.0, 1.0, 1.0, 0.07]);
        }
        glass::keycap(w.stat_font, x + 8.0, y + 7.0, "P");
        glass::text(w.stat_font, x + 38.0, y + (bh - glass::text_h(w.stat_font, text)) / 2.0, text, glass::TEXT);
    }
    if let Some((x, y, width, h, text)) = popup_rect(w) {
        glass::tinted_panel(x, y, width, h, 12.0, [0.047, 0.086, 0.172, 0.9], [1.0, 0.79, 0.3, 0.6]);
        let th = glass::text_h(font, &text);
        glass::text(font, x + 14.0, y + (h - th) / 2.0, &text, [1.0, 0.93, 0.72, 1.0]);
    }
    if w.diplomacy_open {
        let h = panel_height(w);
        glass::panel(PANEL_X, PANEL_TOP, PANEL_W, h);
        let mut y = PANEL_TOP + 8.0;
        glass::text(font, PANEL_X + 16.0, y, "Players and alliances", glass::GOLD);
        let sub = format!("{} fish each side; you have {}. Allies don't fight and win together.", crate::alliance::ALLIANCE_FISH, crate::alliance::fish_of(w, me));
        glass::text(w.stat_font, PANEL_X + 16.0, y + 26.0, &sub, glass::DIM);
        y = PANEL_TOP + ROW_H * 1.5;
        for p in panel_players(w) {
            let pl = &w.players[&p];
            let alive = crate::alliance::alive(w, p);
            let a = if alive { 1.0 } else { 0.45 };
            let c = pl.glcolor;
            let mid = y + ROW_H / 2.0;
            glass::fill(PANEL_X + 16.0, mid - 7.0, 14.0, 14.0, 3.0, [c[0], c[1], c[2], a]);
            glass::text(font, PANEL_X + 38.0, mid - glass::text_h(font, &pl.name) / 2.0, &pl.name, [glass::TEXT[0], glass::TEXT[1], glass::TEXT[2], a]);
            if pl.kind == PlayerKind::Ai && !w.is_server() {
                let t = format!("AI: {}", pl.ai_level.label());
                glass::text(w.stat_font, PANEL_X + LEVEL_X + 8.0, mid - glass::text_h(w.stat_font, &t) / 2.0, &t, [glass::DIM[0], glass::DIM[1], glass::DIM[2], a]);
            }
            let status = if !alive {
                Some(("out of the game".to_string(), [0.6, 0.6, 0.65, 1.0]))
            } else if crate::alliance::allied(w, Some(me), Some(p)) {
                Some(("Allied".to_string(), [0.5, 1.0, 0.6, 1.0]))
            } else if let Some(o) = w.offers.iter().find(|o| o.0 == me && o.1 == p) {
                let left = crate::alliance::OFFER_MS.saturating_sub(now.wrapping_sub(o.2)).div_ceil(1000);
                Some((format!("offer sent ({}s)", left), [1.0, 0.85, 0.5, 1.0]))
            } else {
                None
            };
            if let Some((t, col)) = status {
                glass::text(w.stat_font, PANEL_X + PANEL_W - 190.0, mid - glass::text_h(w.stat_font, &t) / 2.0, &t, [col[0], col[1], col[2], 1.0]);
            }
            y += ROW_H;
        }
    }
    for b in ally_buttons(w) {
        b.draw(w);
    }
}

// --- alerts -------------------------------------------------------------

const ALERT_TOP: f64 = MARGIN + STRIP_H + 8.0;
const ALERT_H: f64 = 50.0;

/// Where the alert box is (x, top, width), top-left origin, if one shows.
fn alert_box(w: &World) -> Option<(f64, f64, f64, &crate::alerts::Alert)> {
    let a = crate::alerts::showing(w)?;
    let tw = glass::text_w(w.chat_font, a.text()).max(glass::text_w(w.stat_font, ALERT_HINT));
    Some((MARGIN, ALERT_TOP, 46.0 + tw + 14.0 + 22.0 + 12.0, a))
}

const ALERT_HINT: &str = "Click to look";

/// The alert (top left, under what you have): red for an igloo being
/// taken, orange for an attack; it blinks for its first seconds.
pub fn draw_alert(w: &World) {
    let Some((x, top, bw, a)) = alert_box(w) else { return };
    let age = crate::sdl::ticks().wrapping_sub(a.at);
    let fade = (1.0 - (age as f32 - (crate::alerts::SHOW_MS - 800) as f32) / 800.0).clamp(0.0, 1.0);
    let blink: f32 = if age < 2400 && (age / 300) % 2 == 0 { 1.25 } else { 1.0 };
    let (col, edge) = match a.kind {
        crate::alerts::AlertKind::Igloo => ([0.55, 0.07, 0.07], [1.0, 0.6, 0.55]),
        crate::alerts::AlertKind::Attack => ([0.47, 0.16, 0.04], [1.0, 0.75, 0.47]),
    };
    let glass_c = [(col[0] * blink).min(1.0), (col[1] * blink).min(1.0), (col[2] * blink).min(1.0), 0.9 * fade];
    glass::tinted_panel(x, top, bw, ALERT_H, 12.0, glass_c, [edge[0], edge[1], edge[2], 0.6 * fade]);
    glass::icon(Icon::Warning, x + 12.0, top + (ALERT_H - 24.0) / 2.0, 24.0, [1.0, 0.79, 0.3, fade]);
    glass::text(w.chat_font, x + 46.0, top + 5.0, a.text(), [1.0, 0.96, 0.9, fade]);
    glass::text(w.stat_font, x + 46.0, top + 28.0, ALERT_HINT, [1.0, 0.85, 0.72, fade]);
    if fade > 0.5 {
        glass::keycap(w.stat_font, x + bw - 12.0 - 22.0, top + (ALERT_H - 22.0) / 2.0, "J");
    }
}

/// Is the mouse on the alert?  (top-left origin)
pub fn alert_at(w: &World, pos: (i32, i32)) -> bool {
    alert_box(w).is_some_and(|(x, top, bw, _)| glass::hit(pos, x, top, bw, ALERT_H))
}

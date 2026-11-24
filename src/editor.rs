//! The map editor (Map editor in the main menu): paint snow, ice and
//! hills, place trees, ice pinnacles, fish holes, crystals, igloos and
//! power-ups, set the map's name and victory, save it among the user's
//! maps, and try it out in a single player game.

use crate::backgrounds;
use crate::editmap::{EditMap, Ground, Music, Power, Res, Victory};
use crate::gfx::{self, Tex};
use crate::image::Image;
use crate::rng::Rng;
use crate::sdl::{self, Rect};
use crate::ui::{self, Action, DRect, Ev, Input, Page, View, Widget};
use crate::world::IGLOO_SIZE;

pub enum Exit {
    /// back to the main menu
    Back,
    /// play this map (file name) in a single player game, then come back
    Test(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Snow,
    Ice,
    Raise,
    Lower,
    Smooth,
    Tree,
    Pinnacle,
    Fish,
    Crystal,
    Erase,
    Igloo,
    Power(Power),
}

impl Tool {
    fn hint(self) -> &'static str {
        match self {
            Tool::Snow | Tool::Ice => "Paint with the left button.",
            Tool::Raise | Tool::Lower | Tool::Smooth => "Hold the left button to shape the hills.",
            Tool::Tree => "Paint trees on snow (right button removes things).",
            Tool::Pinnacle => "Paint ice pinnacles on ice (right button removes things).",
            Tool::Fish | Tool::Crystal => "Paint resources for workers to gather (right button removes things).",
            Tool::Erase => "Remove trees, pinnacles, fish, crystals and power-ups.",
            Tool::Igloo => "Click to place an igloo (one per player); right-click one to remove it.",
            Tool::Power(_) => "Click: add a power-up, right-click: remove. In game: this many drop each minute, at random.",
        }
    }
}

#[derive(PartialEq, Eq)]
enum Mode {
    Choose,
    Paint,
    Settings,
}

pub struct Editor {
    map: Option<EditMap>,
    mode: Mode,
    tool: Tool,
    brush: i32,
    /// the tile at the middle of the view, and pixels per tile
    cam: (f64, f64),
    zoom: f64,
    preview: Image,
    tex: Option<Tex>,
    dirty: bool,
    territories: Vec<u8>,
    undo: Vec<Ground>,
    /// the map changed since it was saved
    changed: bool,
    /// a mouse button painting (1 left, 3 right) and the last tile painted
    stroke: Option<u8>,
    last_tile: Option<(i32, i32)>,
    /// panning with the middle button: start mouse and camera
    pan: Option<((i32, i32), (f64, f64))>,
    mouse: (i32, i32),
    /// a click on a button this frame (screen pixels)
    click: Option<(i32, i32)>,
    status: String,
    status_time: u32,
    last_escape: u32,
    rng: Rng,
    /// the Map settings page (name and description boxes)
    settings: Page,
    /// the chooser's maps: (file, name)
    maps: Vec<(String, String)>,
    pending: Option<Pending>,
    /// A map (file, name) waiting for "yes, delete it".
    confirm_delete: Option<(String, String)>,
    /// The map's music is playing (Listen in the map settings).
    listening: bool,
    /// The last music file dropped (kept while flipping through the tunes).
    custom_music: Option<Music>,
    /// The page of the map list shown (`MAPS_PER_PAGE` a page).
    map_page: usize,
}

/// Arrow-key and pan speed: tiles per second at zoom 1.
const PAN_PX_PER_SEC: f64 = 600.0;

fn tool_color(r: Res) -> Option<[u8; 3]> {
    match r {
        Res::None => None,
        Res::Tree => Some([30, 120, 50]),
        Res::Pinnacle => Some([70, 130, 190]),
        Res::Fish => Some([20, 50, 150]),
        Res::Crystal => Some([200, 40, 200]),
    }
}

/// Distinct tints for the igloos' territories.
fn team_color(i: usize) -> [u8; 3] {
    const C: [[u8; 3]; 12] = [
        [230, 60, 60],
        [60, 110, 230],
        [60, 180, 80],
        [230, 170, 40],
        [160, 70, 200],
        [40, 190, 190],
        [230, 110, 170],
        [130, 130, 130],
        [150, 100, 50],
        [100, 200, 40],
        [40, 60, 140],
        [200, 200, 60],
    ];
    C[i % C.len()]
}

/// Maps shown on a page of the editor's map list.
const MAPS_PER_PAGE: usize = 12;

/// The file of one of the player's own maps (made or changed in the
/// editor), if `file` is one; the game's own maps are never deleted.
fn own_map_file(file: &str) -> Option<std::path::PathBuf> {
    if file.is_empty() || file.contains('/') || file.contains('\\') || file.contains("..") {
        return None;
    }
    let p = std::path::Path::new(&crate::settings::user_maps_dir()).join(format!("{}.zip", file));
    p.is_file().then_some(p)
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

impl Editor {
    pub fn new() -> Editor {
        Editor {
            map: None,
            mode: Mode::Choose,
            tool: Tool::Snow,
            brush: 3,
            cam: (0.0, 0.0),
            zoom: 8.0,
            preview: Image::new(1, 1),
            tex: None,
            dirty: true,
            territories: Vec::new(),
            undo: Vec::new(),
            changed: false,
            stroke: None,
            last_tile: None,
            pan: None,
            mouse: (0, 0),
            click: None,
            status: String::new(),
            status_time: 0,
            last_escape: 0,
            rng: Rng::from_time(),
            settings: Page::default(),
            maps: Vec::new(),
            pending: None,
            confirm_delete: None,
            listening: false,
            custom_music: None,
            map_page: 0,
        }
    }

    fn say(&mut self, text: &str) {
        self.status = text.to_string();
        self.status_time = sdl::ticks();
    }

    /// Start editing `m`.
    fn edit(&mut self, m: EditMap) {
        if let Some(t) = self.tex.take() {
            gfx::free_texture(&t);
        }
        self.cam = (m.w as f64 / 2.0, m.h as f64 / 2.0);
        self.preview = Image::new(m.w as usize, m.h as usize);
        self.map = Some(m);
        self.undo.clear();
        self.changed = false;
        self.dirty = true;
        self.territories.clear();
        self.mode = Mode::Paint;
        self.fit_zoom();
    }

    fn view_rect(&self) -> Rect {
        let (sw, sh) = gfx::screen_size();
        let tb = self.toolbar_width();
        Rect::new(tb, 0, sw - tb, sh)
    }

    /// The left toolbar's width in pixels.
    fn toolbar_width(&self) -> i32 {
        (160.0 * self.ui_scale()) as i32
    }

    /// UI size factor (the menus are made for 480 pixels high).
    fn ui_scale(&self) -> f64 {
        let (_, sh) = gfx::screen_size();
        (sh as f64 / 480.0).max(1.0)
    }

    fn fit_zoom(&mut self) {
        if let Some(m) = &self.map {
            let v = self.view_rect();
            self.zoom = ((v.w as f64 / m.w as f64).min((v.h as f64 - 40.0) / m.h as f64) * 0.95).max(1.0);
        }
    }

    fn tile_at(&self, p: (i32, i32)) -> (i32, i32) {
        let v = self.view_rect();
        let tx = (p.0 - v.x - v.w / 2) as f64 / self.zoom + self.cam.0;
        let ty = (p.1 - v.y - v.h / 2) as f64 / self.zoom + self.cam.1;
        (tx.floor() as i32, ty.floor() as i32)
    }

    fn screen_of(&self, tx: f64, ty: f64) -> (f64, f64) {
        let v = self.view_rect();
        ((v.x + v.w / 2) as f64 + (tx - self.cam.0) * self.zoom, (v.y + v.h / 2) as f64 + (ty - self.cam.1) * self.zoom)
    }

    // ---------------------------------------------------------------------
    // painting
    // ---------------------------------------------------------------------

    fn begin_stroke(&mut self, button: u8) {
        if let Some(m) = &self.map {
            self.undo.push(m.g.clone());
            if self.undo.len() > 40 {
                self.undo.remove(0);
            }
        }
        self.stroke = Some(button);
        self.last_tile = None;
    }

    /// Apply the tool at the mouse.  `first`: the button was just pressed
    /// (igloos and power-ups are placed on a click only); `dt`: ms since the
    /// last frame (the hill tools work over time).
    fn apply(&mut self, first: bool, dt: u32) {
        let Some(button) = self.stroke else { return };
        let (tx, ty) = self.tile_at(self.mouse);
        let moved = self.last_tile != Some((tx, ty));
        let continuous = matches!(self.tool, Tool::Raise | Tool::Lower | Tool::Smooth);
        if !first && !moved && !continuous {
            return;
        }
        self.last_tile = Some((tx, ty));
        let erase = button == 3;
        let tool = self.tool;
        let brush = self.brush;
        let mut message = None;
        let Some(m) = self.map.as_mut() else { return };
        if m.idx(tx, ty).is_none() {
            return;
        }
        let before = m.g.clone();
        // single-click tools
        if erase && (tool == Tool::Igloo || matches!(tool, Tool::Power(_))) {
            if first {
                if let Some(i) = m.igloo_at(tx, ty) {
                    m.g.igloos.remove(i);
                }
                m.g.powerups.retain(|&(_, x, y)| (x, y) != (tx, ty));
            }
        } else if tool == Tool::Igloo {
            if first && m.igloo_at(tx, ty).is_none() {
                if let Err(e) = m.add_igloo(tx - IGLOO_SIZE / 2, ty - IGLOO_SIZE / 2) {
                    message = Some(e);
                }
            }
        } else if let Tool::Power(p) = tool {
            if first {
                if m.igloo_at(tx, ty).is_some() {
                    message = Some("Not on an igloo".to_string());
                } else {
                    m.g.powerups.retain(|&(_, x, y)| (x, y) != (tx, ty));
                    m.g.powerups.push((p, tx, ty));
                }
            }
        } else {
            // brush tools
            let r = brush - 1;
            let density = if r == 0 { 1.0 } else { 0.3 };
            let smooth_src = m.g.height.clone();
            for yy in ty - r..=ty + r {
                for xx in tx - r..=tx + r {
                    let d = (((xx - tx).pow(2) + (yy - ty).pow(2)) as f64).sqrt();
                    if d > r as f64 + 0.5 {
                        continue;
                    }
                    let Some(i) = m.idx(xx, yy) else { continue };
                    let under_igloo = m.igloo_at(xx, yy).is_some();
                    let falloff = 1.0 - d / (r as f64 + 1.0);
                    if erase || tool == Tool::Erase {
                        m.g.res[i] = Res::None;
                        m.g.powerups.retain(|&(_, x, y)| (x, y) != (xx, yy));
                        continue;
                    }
                    match tool {
                        Tool::Snow => {
                            m.g.ice[i] = false;
                            if m.g.res[i] == Res::Pinnacle {
                                m.g.res[i] = Res::None;
                            }
                        }
                        Tool::Ice if !under_igloo => {
                            m.g.ice[i] = true;
                            if m.g.res[i] == Res::Tree {
                                m.g.res[i] = Res::None;
                            }
                        }
                        Tool::Raise | Tool::Lower => {
                            let step = (dt.max(16) as f64 * 0.12 * falloff).max(1.0) as i32;
                            let h = m.g.height[i] as i32 + if tool == Tool::Raise { step } else { -step };
                            m.g.height[i] = h.clamp(0, 255) as u8;
                        }
                        Tool::Smooth => {
                            let mut sum = 0;
                            let mut n = 0;
                            for oy in -1..=1 {
                                for ox in -1..=1 {
                                    if let Some(j) = m.idx(xx + ox, yy + oy) {
                                        sum += smooth_src[j] as i32;
                                        n += 1;
                                    }
                                }
                            }
                            let avg = sum / n.max(1);
                            let h = m.g.height[i] as i32;
                            m.g.height[i] = (h + ((avg - h) as f64 * falloff * 0.5).round() as i32).clamp(0, 255) as u8;
                        }
                        Tool::Tree | Tool::Pinnacle | Tool::Fish | Tool::Crystal if !under_igloo => {
                            let (res, ok) = match tool {
                                Tool::Tree => (Res::Tree, !m.g.ice[i]),
                                Tool::Pinnacle => (Res::Pinnacle, m.g.ice[i]),
                                Tool::Fish => (Res::Fish, true),
                                _ => (Res::Crystal, true),
                            };
                            if ok && m.g.res[i] == Res::None && self.rng.random() < density {
                                m.g.res[i] = res;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if first && matches!(tool, Tool::Tree) && m.g.ice[m.idx(tx, ty).unwrap()] {
                message = Some("Trees grow on snow (use ice pinnacles on ice)".into());
            }
            if first && matches!(tool, Tool::Pinnacle) && !m.g.ice[m.idx(tx, ty).unwrap()] {
                message = Some("Ice pinnacles stand on ice (paint ice first)".into());
            }
        }
        if m.g != before {
            self.changed = true;
            self.dirty = true;
        }
        if let Some(t) = message {
            self.say(&t);
        }
    }

    fn undo(&mut self) {
        if let (Some(m), Some(g)) = (self.map.as_mut(), self.undo.pop()) {
            m.g = g;
            self.dirty = true;
            self.changed = true;
            self.say("Undone");
        } else {
            self.say("Nothing to undo");
        }
    }

    // ---------------------------------------------------------------------
    // pictures
    // ---------------------------------------------------------------------

    /// The map as one pixel per tile: snow / ice shaded by height, the
    /// territories tinted, resources as coloured dots.
    fn render_preview(&mut self) {
        let Some(m) = &self.map else { return };
        if self.territories.len() != (m.w * m.h) as usize || self.dirty {
            self.territories = m.territories();
        }
        let (w, h) = (m.w as usize, m.h as usize);
        if self.preview.w != w || self.preview.h != h {
            self.preview = Image::new(w, h);
        }
        let has_igloos = !m.g.igloos.is_empty();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let hv = m.g.height[i] as f64 / 255.0;
                // light from the top left
                let up = if x > 0 && y > 0 { m.g.height[i - w - 1] as f64 } else { m.g.height[i] as f64 };
                let shade = (1.0 + (m.g.height[i] as f64 - up) / 255.0 * 2.5).clamp(0.6, 1.25);
                let mut c = if m.g.ice[i] {
                    [110.0 + 60.0 * hv, 190.0 + 40.0 * hv, 235.0]
                } else {
                    [205.0 + 50.0 * hv, 212.0 + 43.0 * hv, 225.0 + 30.0 * hv]
                };
                for v in c.iter_mut() {
                    *v *= shade;
                }
                if has_igloos {
                    let t = team_color(self.territories[i] as usize);
                    let border = (x + 1 < w && self.territories[i + 1] != self.territories[i]) || (y + 1 < h && self.territories[i + w] != self.territories[i]);
                    let a = if border { 0.45 } else { 0.10 };
                    for k in 0..3 {
                        c[k] = c[k] * (1.0 - a) + t[k] as f64 * a;
                    }
                }
                if let Some(rc) = tool_color(m.g.res[i]) {
                    c = [rc[0] as f64, rc[1] as f64, rc[2] as f64];
                }
                self.preview.set(x, y, [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, 255]);
            }
        }
        match self.tex {
            Some(t) if t.w == w as i32 && t.h == h as i32 => gfx::update_texture(&t, &self.preview),
            _ => {
                if let Some(t) = self.tex.take() {
                    gfx::free_texture(&t);
                }
                self.tex = gfx::texture_from_image(&self.preview).ok();
                if let Some(t) = &self.tex {
                    gfx::set_texture_smooth(t, false);
                }
            }
        }
        self.dirty = false;
    }

    /// The map list picture (256 x 256).
    fn icon(&self) -> Image {
        let Some(m) = &self.map else { return Image::new(256, 256) };
        let mut out = Image::new(256, 256);
        let s = 256.0 / m.w.max(m.h) as f64;
        for y in 0..256 {
            for x in 0..256 {
                let (tx, ty) = ((x as f64 / s) as usize, (y as f64 / s) as usize);
                if tx < self.preview.w && ty < self.preview.h {
                    out.set(x, y, self.preview.get(tx, ty));
                }
            }
        }
        for (i, &(ix, iy)) in m.g.igloos.iter().enumerate() {
            let c = team_color(i);
            let (x0, y0) = ((ix as f64 * s) as i32, (iy as f64 * s) as i32);
            let size = ((IGLOO_SIZE as f64 * s) as i32).max(6);
            for y in y0..y0 + size {
                for x in x0..x0 + size {
                    if (0..256).contains(&x) && (0..256).contains(&y) {
                        let edge = x == x0 || y == y0 || x == x0 + size - 1 || y == y0 + size - 1;
                        out.set(x as usize, y as usize, if edge { [30, 30, 30, 255] } else { [c[0], c[1], c[2], 255] });
                    }
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------------
    // the screen
    // ---------------------------------------------------------------------

    fn font(&self, size: f64) -> gfx::Font {
        ui::font(((size * self.ui_scale()).round() as u32).max(6))
    }

    fn text(&self, x: i32, y: i32, size: f64, t: &str, color: [f32; 4]) {
        self.font(size).draw_screen(x, y, t, color);
    }

    /// A button; true when clicked this frame.
    fn button(&mut self, r: Rect, label: &str, active: bool) -> bool {
        let inside = |p: (i32, i32)| p.0 >= r.x && p.1 >= r.y && p.0 < r.x + r.w && p.1 < r.y + r.h;
        let hover = inside(self.mouse);
        let v = View { scale: self.ui_scale(), off_x: 0.0, off_y: 0.0 };
        let name = if active {
            "button_down.png"
        } else if hover {
            "button_hover.png"
        } else {
            "button.png"
        };
        ui::slice9(&v, name, r);
        if active {
            outline(r.x as f64 - 1.0, r.y as f64 - 1.0, r.w as f64 + 2.0, r.h as f64 + 2.0, [1.0, 0.85, 0.1, 1.0]);
        }
        let f = self.font(13.0);
        let (tw, th) = f.size_of(label);
        f.draw_screen(r.x + (r.w - tw) / 2, r.y + (r.h - th) / 2, label, ui::TEXT_COLOR);
        if self.click.map(inside).unwrap_or(false) {
            self.click = None;
            return true;
        }
        false
    }

    fn draw_paint(&mut self) {
        if self.dirty {
            self.render_preview();
        }
        let (sw, sh) = gfx::screen_size();
        let k = self.ui_scale();
        let vr = self.view_rect();
        gfx::fill_screen_rect(vr, (40, 46, 60, 255));
        let Some(m) = &self.map else { return };
        let (mw, mh) = (m.w, m.h);
        gfx::set_clip(Some(vr));
        let (x0, y0) = self.screen_of(0.0, 0.0);
        if let Some(t) = &self.tex {
            gfx::blit(t, None, Rect::new(x0.round() as i32, y0.round() as i32, (mw as f64 * self.zoom).round() as i32, (mh as f64 * self.zoom).round() as i32), 1.0);
        }
        // pictures of things when zoomed in
        if self.zoom >= 10.0 {
            let (ax, ay) = self.tile_at((vr.x, vr.y));
            let (bx, by) = self.tile_at((vr.x + vr.w, vr.y + vr.h));
            let trees = gfx::texture_sized("data/trees.png", 256);
            let pins = gfx::texture_sized("data/icepinnacles.png", 256);
            let fish = gfx::texture_sized("data/fish.png", 32);
            let crystal = gfx::texture_sized("data/crystal.png", 32);
            for ty in (ay - 2).max(0)..=(by + 2).min(mh - 1) {
                for tx in (ax - 2).max(0)..=(bx + 2).min(mw - 1) {
                    let i = (ty * mw + tx) as usize;
                    let (sx, sy) = self.screen_of(tx as f64, ty as f64);
                    let z = self.zoom;
                    let (tex, w, h) = match m.g.res[i] {
                        Res::Tree => (trees.sub(64 * ((tx + ty) % 2), 64 * ((tx * 7 + ty) % 2), 64, 64), 2.0, 2.0),
                        Res::Pinnacle => (pins.sub(64 * ((tx + ty) % 2), 0, 64, 64), 2.0, 2.0),
                        Res::Fish => (fish, 1.0, 1.0),
                        Res::Crystal => (crystal, 1.0, 1.0),
                        Res::None => continue,
                    };
                    let dst = Rect::new((sx - (w - 1.0) / 2.0 * z) as i32, (sy - (h - 1.0) * z) as i32, (w * z) as i32, (h * z) as i32);
                    gfx::blit(&tex, None, dst, 1.0);
                }
            }
        }
        let igloo = gfx::texture_sized("data/igloo.png", 128);
        for (i, &(ix, iy)) in m.g.igloos.iter().enumerate() {
            let (sx, sy) = self.screen_of(ix as f64, iy as f64);
            let size = (IGLOO_SIZE as f64 * self.zoom) as i32;
            // at least 28 pixels, so it can be seen zoomed out
            let shown = size.max(28);
            let (ix0, iy0) = (sx as i32 + size / 2 - shown / 2, sy as i32 + size / 2 - shown / 2);
            let c = team_color(i);
            outline(ix0 as f64 - 2.0, iy0 as f64 - 2.0, shown as f64 + 4.0, shown as f64 + 4.0, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]);
            gfx::blit(&igloo, None, Rect::new(ix0, iy0, shown, shown), 1.0);
            // the player number above it
            let label = format!("{}", i + 1);
            let f = self.font(11.0);
            let (tw, th) = f.size_of(&label);
            let (lx, ly) = (ix0 + shown / 2 - tw / 2, iy0 - th - 4);
            gfx::fill_screen_rect(Rect::new(lx - 4, ly, tw + 8, th + 2), (c[0], c[1], c[2], 230));
            f.draw_screen(lx, ly, &label, [1.0; 4]);
        }
        for &(p, x, y) in &m.g.powerups {
            let (sx, sy) = self.screen_of(x as f64 - 0.2, y as f64 - 0.2);
            let size = (self.zoom * 1.4).max(10.0) as i32;
            gfx::blit(&gfx::texture(p.image()), None, Rect::new(sx as i32, sy as i32, size, size), 1.0);
        }
        // the brush outline
        if self.mouse.0 >= vr.x {
            let (tx, ty) = self.tile_at(self.mouse);
            let r = match self.tool {
                Tool::Igloo => {
                    let (sx, sy) = self.screen_of((tx - IGLOO_SIZE / 2) as f64, (ty - IGLOO_SIZE / 2) as f64);
                    let s = IGLOO_SIZE as f64 * self.zoom;
                    let ok = m.igloo_fits(tx - IGLOO_SIZE / 2, ty - IGLOO_SIZE / 2);
                    let c = if ok { [0.2, 1.0, 0.3, 0.9] } else { [1.0, 0.2, 0.2, 0.9] };
                    outline(sx, sy, s, s, c);
                    None
                }
                Tool::Power(_) => Some(0),
                _ => Some(self.brush - 1),
            };
            if let Some(r) = r {
                let (sx, sy) = self.screen_of((tx - r) as f64, (ty - r) as f64);
                let s = (2 * r + 1) as f64 * self.zoom;
                outline(sx, sy, s, s, [1.0, 1.0, 1.0, 0.8]);
            }
        }
        gfx::set_clip(None);

        // the status line
        let line = if !self.status.is_empty() && sdl::ticks().wrapping_sub(self.status_time) < 5000 {
            self.status.clone()
        } else {
            let (tx, ty) = self.tile_at(self.mouse);
            let pos = if self.mouse.0 >= vr.x && tx >= 0 && ty >= 0 && tx < mw && ty < mh { format!("{}, {}   ", tx, ty) } else { String::new() };
            format!("{}{}   Wheel: zoom   Middle drag / arrows: move   Ctrl+Z: undo", pos, self.tool.hint())
        };
        let fh = (22.0 * k) as i32;
        gfx::fill_screen_rect(Rect::new(vr.x, sh - fh, sw - vr.x, fh), (0, 0, 0, 160));
        self.text(vr.x + 8, sh - fh + (3.0 * k) as i32, 12.0, &line, [1.0, 1.0, 1.0, 1.0]);
        let title = format!("{}{}  ({} x {})", m.name, if self.changed { " *" } else { "" }, mw, mh);
        self.text(vr.x + 8, (6.0 * k) as i32, 14.0, &title, [1.0, 1.0, 1.0, 1.0]);

        self.draw_toolbar();
    }

    fn draw_toolbar(&mut self) {
        let (_, sh) = gfx::screen_size();
        let k = self.ui_scale();
        let tbw = self.toolbar_width();
        let v = View { scale: k, off_x: 0.0, off_y: 0.0 };
        ui::panel_bg(&v, Rect::new(0, 0, tbw, sh));
        let pad = (6.0 * k) as i32;
        let bw = tbw - 2 * pad;
        let bh = (20.0 * k) as i32;
        let gap = (3.0 * k) as i32;
        let mut y = pad;
        let label = |s: &Self, y: &mut i32, t: &str| {
            s.text(pad, *y, 12.0, t, ui::TEXT_COLOR);
            *y += (16.0 * k) as i32;
        };
        let groups: [(&str, &[(&str, Tool)]); 4] = [
            ("Ground", &[("Snow", Tool::Snow), ("Ice", Tool::Ice)]),
            ("Hills", &[("Raise", Tool::Raise), ("Lower", Tool::Lower), ("Smooth", Tool::Smooth)]),
            ("Things", &[("Trees", Tool::Tree), ("Pinnacles", Tool::Pinnacle), ("Fish", Tool::Fish), ("Crystal", Tool::Crystal), ("Eraser", Tool::Erase)]),
            ("Players & power-ups", &[("Igloo", Tool::Igloo), ("Rapid fire", Tool::Power(Power::DoubleSnowball)), ("Range", Tool::Power(Power::Range)), ("Defense", Tool::Power(Power::Defense))]),
        ];
        for (title, tools) in groups {
            label(self, &mut y, title);
            // two buttons per row
            for pair in tools.chunks(2) {
                let w = if pair.len() == 2 { (bw - gap) / 2 } else { bw };
                for (j, (name, tool)) in pair.iter().enumerate() {
                    let r = Rect::new(pad + j as i32 * (w + gap), y, w, bh);
                    if self.button(r, name, self.tool == *tool) {
                        self.tool = *tool;
                    }
                }
                y += bh + gap;
            }
            y += gap;
        }
        // brush size
        label(self, &mut y, &format!("Brush size: {}", self.brush));
        let w = (bw - gap) / 2;
        if self.button(Rect::new(pad, y, w, bh), "-", false) {
            self.brush = (self.brush - 1).max(1);
        }
        if self.button(Rect::new(pad + w + gap, y, w, bh), "+", false) {
            self.brush = (self.brush + 1).min(12);
        }
        // the map's buttons at the bottom
        let rows = [("Map settings", 0), ("Save", 1), ("Test play", 2), ("Exit", 3)];
        let mut by = sh - pad - 4 * (bh + gap);
        for (name, n) in rows {
            if self.button(Rect::new(pad, by, bw, bh), name, false) {
                match n {
                    0 => self.open_settings(),
                    1 => self.pending = Some(Pending::Save),
                    2 => self.pending = Some(Pending::Test),
                    _ => self.pending = Some(Pending::Exit),
                }
            }
            by += bh + gap;
        }
    }
}

fn outline(x: f64, y: f64, w: f64, h: f64, c: [f32; 4]) {
    let c8 = |v: f32| (v * 255.0) as u8;
    let col = (c8(c[0]), c8(c[1]), c8(c[2]), c8(c[3]));
    let (x, y, w, h) = (x as i32, y as i32, w.max(1.0) as i32, h.max(1.0) as i32);
    gfx::fill_screen_rect(Rect::new(x, y, w, 2), col);
    gfx::fill_screen_rect(Rect::new(x, y + h - 2, w, 2), col);
    gfx::fill_screen_rect(Rect::new(x, y, 2, h), col);
    gfx::fill_screen_rect(Rect::new(x + w - 2, y, 2, h), col);
}

/// Something a toolbar button asked for (done after drawing).
#[derive(Clone, Copy, PartialEq)]
enum Pending {
    Save,
    Test,
    Exit,
}

impl Editor {
    // ---------------------------------------------------------------------
    // the map settings page and the chooser (640 x 480 design screens)
    // ---------------------------------------------------------------------

    fn design_view(&self) -> View {
        let (sw, sh) = gfx::screen_size();
        let (scale, off_x, off_y) = backgrounds::design_view(sw, sh);
        View { scale, off_x, off_y }
    }

    fn to_design(&self, p: (i32, i32)) -> (i32, i32) {
        let v = self.design_view();
        (((p.0 as f64 - v.off_x) / v.scale).floor() as i32, ((p.1 as f64 - v.off_y) / v.scale).floor() as i32)
    }

    /// A button at design coordinates.
    fn dbutton(&mut self, x: i32, y: i32, w: i32, h: i32, label: &str, active: bool) -> bool {
        let r = self.design_view().to_screen(&DRect { x, y, w, h });
        self.button(r, label, active)
    }

    fn dtext(&self, x: i32, y: i32, size: f64, t: &str, color: [f32; 4]) {
        let v = self.design_view();
        let r = v.to_screen(&DRect { x, y, w: 1, h: 1 });
        ui::font(((size * v.scale).round() as u32).max(6)).draw_screen(r.x, r.y, t, color);
    }

    fn open_settings(&mut self) {
        let Some(m) = &self.map else { return };
        let mut page = Page::default();
        let mut name = Input::new(&m.name, 360, 16);
        name.disallowed.clear();
        name.max_len = 40;
        name.rect.x = 190;
        name.rect.y = 88;
        let mut desc = Input::new(&m.description, 360, 14);
        desc.disallowed.clear();
        desc.max_len = 120;
        desc.rect.x = 190;
        desc.rect.y = 130;
        page.add(Widget::Input(name));
        page.add(Widget::Input(desc));
        self.settings = page;
        self.mode = Mode::Settings;
    }

    fn close_settings(&mut self) {
        let mut vals = Vec::new();
        for w in &self.settings.widgets {
            if let Widget::Input(i) = w {
                vals.push(i.value.clone());
            }
        }
        if let (Some(m), [name, desc]) = (self.map.as_mut(), vals.as_slice()) {
            if *name != m.name || *desc != m.description {
                self.changed = true;
            }
            if !name.trim().is_empty() {
                m.name = name.trim().to_string();
            }
            m.description = desc.trim().to_string();
        }
        self.mode = Mode::Paint;
    }

    fn draw_settings(&mut self) {
        let (sw, sh) = gfx::screen_size();
        gfx::fill_screen_rect(Rect::new(0, 0, sw, sh), (0, 0, 0, 150));
        let v = self.design_view();
        ui::panel_bg(&v, v.to_screen(&DRect { x: 50, y: 40, w: 540, h: 400 }));
        let black = ui::TEXT_COLOR;
        self.dtext(70, 55, 20.0, "Map settings", black);
        self.dtext(70, 95, 16.0, "Name:", black);
        self.dtext(70, 135, 14.0, "Description:", black);
        self.settings.draw(&v);
        let Some(m) = &self.map else { return };
        let (victory, amount, workers, snowballers) = (m.victory, m.amount, m.workers, m.snowballers);
        self.dtext(70, 180, 16.0, "Victory:", black);
        let opts = [("Take every igloo", Victory::Conquer), ("Gather crystal", Victory::Crystal), ("Gather fish", Victory::Fish)];
        let mut new_victory = None;
        for (i, (label, vic)) in opts.iter().enumerate() {
            if self.dbutton(190 + i as i32 * 128, 176, 122, 26, label, victory == *vic) {
                new_victory = Some(*vic);
            }
        }
        let mut d_amount = 0;
        if victory != Victory::Conquer {
            self.dtext(70, 222, 16.0, &format!("Loads to win: {}", amount), black);
            if self.dbutton(250, 218, 34, 26, "-", false) {
                d_amount = -5;
            }
            if self.dbutton(290, 218, 34, 26, "+", false) {
                d_amount = 5;
            }
        }
        self.dtext(70, 262, 16.0, &format!("Workers per igloo: {}", workers), black);
        let mut d_w = 0;
        if self.dbutton(290, 258, 34, 26, "-", false) {
            d_w = -1;
        }
        if self.dbutton(330, 258, 34, 26, "+", false) {
            d_w = 1;
        }
        self.dtext(70, 300, 16.0, &format!("Snowballers per igloo: {}", snowballers), black);
        let mut d_s = 0;
        if self.dbutton(290, 296, 34, 26, "-", false) {
            d_s = -1;
        }
        if self.dbutton(330, 296, 34, 26, "+", false) {
            d_s = 1;
        }
        // the music: the game's tunes, or the map's own (dropped on the window)
        let music = self.map.as_ref().map(|m| m.music.clone()).unwrap_or(Music::Default);
        self.dtext(70, 336, 16.0, "Music:", black);
        let prev = self.dbutton(140, 332, 30, 26, "<", false);
        let next = self.dbutton(174, 332, 30, 26, ">", false);
        self.dtext(212, 337, 13.0, &music.label(), black);
        let listen = self.dbutton(500, 332, 76, 26, if self.listening { "Stop" } else { "Listen" }, self.listening);
        self.dtext(70, 364, 11.0, "Your own music: drop an .ogg file on this window (it is saved inside the map).", [0.2, 0.2, 0.3, 1.0]);
        if prev || next {
            self.cycle_music(if next { 1 } else { -1 });
        }
        if listen {
            self.listen(!self.listening);
        }
        let objectives = self.map.as_ref().map(|m| m.objectives()).unwrap_or_default();
        self.dtext(70, 382, 13.0, &format!("Players see: {}", objectives), [0.2, 0.2, 0.3, 1.0]);
        let done = self.dbutton(270, 404, 100, 28, "Done", false);
        if let Some(m) = self.map.as_mut() {
            let before = (m.victory, m.amount, m.workers, m.snowballers);
            if let Some(vic) = new_victory {
                m.victory = vic;
            }
            m.amount = (m.amount + d_amount).clamp(5, 200);
            m.workers = (m.workers as i32 + d_w).clamp(0, 40) as u16;
            m.snowballers = (m.snowballers as i32 + d_s).clamp(0, 40) as u16;
            if m.workers + m.snowballers == 0 {
                m.snowballers = 1;
            }
            if before != (m.victory, m.amount, m.workers, m.snowballers) {
                self.changed = true;
            }
        }
        if done {
            self.listen(false);
            self.close_settings();
        }
    }

    /// The music choices in order: the usual, the game's tunes, the map's own.
    fn music_choices(&self) -> Vec<Music> {
        let mut v = vec![Music::Default];
        v.extend(crate::editmap::game_tunes().into_iter().filter(|t| t != "main_menu.ogg").map(Music::Track));
        if let Some(Music::Custom { name, data }) = self.map.as_ref().map(|m| &m.music) {
            v.push(Music::Custom { name: name.clone(), data: data.clone() });
        } else if let Some(c) = &self.custom_music {
            v.push(c.clone());
        }
        v
    }

    fn cycle_music(&mut self, by: i32) {
        let choices = self.music_choices();
        let Some(m) = self.map.as_mut() else { return };
        // (keep the map's own music at hand while flipping past it)
        if let Music::Custom { .. } = &m.music {
            self.custom_music = Some(m.music.clone());
        }
        let i = choices.iter().position(|c| *c == m.music).unwrap_or(0) as i32;
        let n = choices.len() as i32;
        m.music = choices[(i + by).rem_euclid(n) as usize].clone();
        self.changed = true;
        if self.listening {
            self.listen(true);
        }
    }

    /// Import a map from the computer (see `editmap::import_map`).
    fn import(&mut self, path: &str) {
        match crate::editmap::import_map(path) {
            Ok((file, name)) => {
                self.maps = crate::menu::list_maps().into_iter().map(|m| (m.filename, m.name)).collect();
                // (show the page it is on)
                if let Some(i) = self.maps.iter().position(|(f, _)| *f == file) {
                    self.map_page = i / MAPS_PER_PAGE;
                }
                self.say(&format!("Imported {}: it is in your maps now (and in the game's map list)", name));
            }
            Err(e) => self.say(&e),
        }
    }

    fn music_dropped(&mut self, path: &str) {
        if self.map.is_none() {
            return;
        }
        match crate::editmap::read_music_file(path) {
            Ok(music) => {
                let label = music.label();
                self.custom_music = Some(music.clone());
                if let Some(m) = self.map.as_mut() {
                    m.music = music;
                }
                self.changed = true;
                self.say(&format!("Map music: {} (saved with the map)", label));
                if self.listening {
                    self.listen(true);
                }
            }
            Err(e) => self.say(&e),
        }
    }

    /// Play (or stop) the map's music, back to the menu tune after.
    fn listen(&mut self, on: bool) {
        if !on {
            if self.listening {
                crate::audio::play(&["main_menu.ogg".to_string()], -1);
            }
            self.listening = false;
            return;
        }
        let names = match self.map.as_ref().map(|m| &m.music) {
            Some(Music::Track(t)) => vec![t.clone()],
            Some(Music::Custom { data, .. }) => {
                let p = std::env::temp_dir().join("snowballz-map-music-preview.ogg");
                if std::fs::write(&p, data).is_err() {
                    self.say("Could not play the music (no room for a temporary file)");
                    return;
                }
                vec![p.to_string_lossy().into_owned()]
            }
            _ => vec!["snowballz2.ogg".to_string()],
        };
        self.listening = crate::audio::play(&names, -1);
        if !self.listening {
            self.say("Could not play the music (is the sound on?)");
        }
    }

    fn draw_choose(&mut self) -> Option<Exit> {
        gfx::clear((0, 0, 0));
        backgrounds::draw("menu", 1.0);
        let v = self.design_view();
        ui::panel_bg(&v, v.to_screen(&DRect { x: 60, y: 30, w: 520, h: 420 }));
        let black = ui::TEXT_COLOR;
        self.dtext(80, 45, 22.0, "Map editor", black);
        self.dtext(80, 85, 16.0, "New map:", black);
        let sizes = [("Small (80 x 80)", 80), ("Medium (120 x 120)", 120), ("Large (160 x 160)", 160)];
        let mut new = None;
        for (i, (label, size)) in sizes.iter().enumerate() {
            if self.dbutton(80 + i as i32 * 162, 108, 156, 28, label, false) {
                new = Some(*size);
            }
        }
        self.dtext(80, 155, 16.0, "Change a map:", black);
        self.dtext(205, 160, 11.0, "(X deletes one of your own maps)", [0.2, 0.2, 0.3, 1.0]);
        let mut open = None;
        let mut delete = None;
        let maps = self.maps.clone();
        let confirming = self.confirm_delete.is_some();
        // pages of maps
        let pages = maps.len().div_ceil(MAPS_PER_PAGE).max(1);
        self.map_page = self.map_page.min(pages - 1);
        if pages > 1 {
            self.dtext(420, 155, 13.0, &format!("Page {} of {}", self.map_page + 1, pages), black);
            if self.dbutton(500, 150, 30, 24, "<", false) && !confirming {
                self.map_page = (self.map_page + pages - 1) % pages;
            }
            if self.dbutton(534, 150, 30, 24, ">", false) && !confirming {
                self.map_page = (self.map_page + 1) % pages;
            }
        }
        let first = self.map_page * MAPS_PER_PAGE;
        for (i, (file, name)) in maps.iter().enumerate().skip(first).take(MAPS_PER_PAGE) {
            let i = i - first;
            let (col, row) = ((i % 2) as i32, (i / 2) as i32);
            let (x, y) = (80 + col * 243, 178 + row * 32);
            // your own maps (not the game's) can be deleted
            let own = own_map_file(file).is_some();
            let w = if own { 205 } else { 237 };
            if self.dbutton(x, y, w, 28, name, false) && !confirming {
                open = Some(file.clone());
            }
            if own && self.dbutton(x + 209, y, 28, 28, "X", false) && !confirming {
                delete = Some((file.clone(), name.clone()));
            }
        }
        if let Some(d) = delete {
            self.confirm_delete = Some(d);
        }
        self.dtext(80, 372, 11.0, &format!("Your maps are saved in {}", crate::settings::user_maps_dir()), [0.2, 0.2, 0.3, 1.0]);
        if let Some((file, name)) = self.confirm_delete.clone() {
            // (a changed copy of one of the game's maps: deleting it brings
            // the original back)
            let original = std::path::Path::new(&format!("maps/{}.zip", file)).exists();
            let q = if original { format!("Delete your changed {}? (the original stays)", name) } else { format!("Delete {}? This can't be undone.", name) };
            self.dtext(80, 388, 13.0, &q, [0.7, 0.0, 0.0, 1.0]);
            let yes = self.dbutton(80, 404, 100, 28, "Delete", false);
            let no = self.dbutton(186, 404, 100, 28, "Cancel", false);
            if yes {
                self.confirm_delete = None;
                match own_map_file(&file).map(std::fs::remove_file) {
                    Some(Ok(())) => {
                        self.maps = crate::menu::list_maps().into_iter().map(|m| (m.filename, m.name)).collect();
                        self.say(&format!("Deleted {}", name));
                    }
                    Some(Err(e)) => self.say(&format!("Could not delete {}: {}", name, e)),
                    None => {}
                }
            } else if no {
                self.confirm_delete = None;
            }
        }
        let import = self.dbutton(330, 404, 140, 28, "Import a map...", false) && !confirming;
        if import {
            match crate::editmap::pick_map_file() {
                Ok(Some(path)) => self.import(&path),
                Ok(None) => {}
                Err(e) => self.say(&e),
            }
        }
        if !confirming && (self.status.is_empty() || sdl::ticks().wrapping_sub(self.status_time) >= 5000) {
            self.dtext(80, 388, 11.0, "Import: pick a map's .zip, or drag it onto this window.", [0.2, 0.2, 0.3, 1.0]);
        }
        let back = self.dbutton(480, 404, 84, 28, "Back", false) && !confirming;
        if let Some(size) = new {
            let mut m = EditMap::new(size);
            m.name = format!("New map {}", size);
            self.edit(m);
            self.changed = true;
        }
        if let Some(f) = open {
            match EditMap::open(&f) {
                Ok(m) => self.edit(m),
                Err(e) => self.say(&format!("Could not open {}: {}", f, e)),
            }
        }
        if self.confirm_delete.is_none() && !self.status.is_empty() && sdl::ticks().wrapping_sub(self.status_time) < 5000 {
            self.dtext(80, 385, 13.0, &self.status.clone(), [0.7, 0.0, 0.0, 1.0]);
        }
        back.then_some(Exit::Back)
    }

    // ---------------------------------------------------------------------
    // saving
    // ---------------------------------------------------------------------

    fn save(&mut self) -> Option<String> {
        self.render_preview();
        let icon = self.icon();
        let m = self.map.as_mut()?;
        match m.save(&icon) {
            Ok(file) => {
                self.changed = false;
                let note = m.problems().map(|p| format!(" (not playable yet: {})", p)).unwrap_or_default();
                self.say(&format!("Saved as {}.zip{}", file, note));
                Some(file)
            }
            Err(e) => {
                self.say(&e);
                None
            }
        }
    }

    // ---------------------------------------------------------------------
    // the loop
    // ---------------------------------------------------------------------

    /// Run the editor until the player leaves it or wants to test a map.
    pub fn run(&mut self) -> Exit {
        sdl::show_cursor(true);
        self.maps = crate::menu::list_maps().into_iter().map(|m| (m.filename, m.name)).collect();
        if self.map.is_some() {
            self.mode = Mode::Paint;
            self.dirty = true;
        }
        let mut clock = crate::clock::Clock::new();
        loop {
            let dt = clock.tick(60).min(100);
            self.click = None;
            for e in gfx::events() {
                if let Some(exit) = self.event(&e) {
                    return exit;
                }
            }
            // the hill tools work while the button is held
            if self.stroke.is_some() && matches!(self.tool, Tool::Raise | Tool::Lower | Tool::Smooth) && self.mode == Mode::Paint {
                self.apply(false, dt);
            }
            // arrow keys / WASD move the view
            if self.mode == Mode::Paint {
                let step = PAN_PX_PER_SEC * dt as f64 / 1000.0 / self.zoom;
                let key = |a, b| sdl::key_pressed(a) || sdl::key_pressed(b);
                if key(sdl::SCAN_LEFT, sdl::SCAN_A) {
                    self.cam.0 -= step;
                }
                if key(sdl::SCAN_RIGHT, sdl::SCAN_D) {
                    self.cam.0 += step;
                }
                if key(sdl::SCAN_UP, sdl::SCAN_W) && sdl::mod_state() & sdl::KMOD_CTRL == 0 {
                    self.cam.1 -= step;
                }
                if key(sdl::SCAN_DOWN, sdl::SCAN_S) && sdl::mod_state() & sdl::KMOD_CTRL == 0 {
                    self.cam.1 += step;
                }
            }
            gfx::clear((0, 0, 0));
            let exit = match self.mode {
                Mode::Choose => self.draw_choose(),
                Mode::Paint => {
                    self.draw_paint();
                    None
                }
                Mode::Settings => {
                    self.draw_paint();
                    self.draw_settings();
                    None
                }
            };
            gfx::present();
            if let Some(e) = exit {
                return e;
            }
            if let Some(p) = self.pending.take() {
                match p {
                    Pending::Save => {
                        self.save();
                    }
                    Pending::Test => {
                        if let Some(problem) = self.map.as_ref().and_then(|m| m.problems()) {
                            self.say(&format!("Can't play yet: {}", problem));
                        } else if let Some(file) = self.save() {
                            return Exit::Test(file);
                        }
                    }
                    Pending::Exit => {
                        if let Some(e) = self.try_leave() {
                            return e;
                        }
                    }
                }
            }
        }
    }

    /// Leave the map (twice if it has unsaved changes).
    fn try_leave(&mut self) -> Option<Exit> {
        let now = sdl::ticks();
        if self.changed && now.wrapping_sub(self.last_escape) > 3000 {
            self.last_escape = now;
            self.say("The map has unsaved changes: press again to leave anyway (or Save first)");
            return None;
        }
        self.map = None;
        self.mode = Mode::Choose;
        self.maps = crate::menu::list_maps().into_iter().map(|m| (m.filename, m.name)).collect();
        None
    }

    fn event(&mut self, e: &sdl::Event) -> Option<Exit> {
        let ctrl = sdl::mod_state() & (sdl::KMOD_CTRL | sdl::KMOD_GUI) != 0;
        match e.kind() {
            sdl::QUIT => crate::quit(0),
            sdl::DROPFILE => {
                // a file dropped on the window: a map to import, or music
                // for the map
                if let Some(path) = e.take_drop_file() {
                    if path.to_lowercase().ends_with(".zip") {
                        self.import(&path);
                    } else if self.map.is_some() {
                        self.music_dropped(&path);
                    } else {
                        self.say("Drop a map (.zip) here to import it");
                    }
                }
            }
            sdl::KEYDOWN if e.key_sym() == sdl::K_F11 => {
                crate::menu::toggle_fullscreen();
                self.fit_zoom();
            }
            sdl::MOUSEMOTION => {
                self.mouse = (e.motion_x(), e.motion_y());
                if self.mode == Mode::Settings {
                    let p = self.to_design(self.mouse);
                    self.settings.handle(&Ev::Motion(p));
                }
                if let Some((start, cam)) = self.pan {
                    self.cam = (cam.0 - (self.mouse.0 - start.0) as f64 / self.zoom, cam.1 - (self.mouse.1 - start.1) as f64 / self.zoom);
                } else if self.stroke.is_some() {
                    self.apply(false, 0);
                }
            }
            sdl::MOUSEBUTTONDOWN => {
                let p = (e.button_x(), e.button_y());
                self.mouse = p;
                match self.mode {
                    Mode::Choose => self.click = Some(p),
                    Mode::Settings => {
                        self.click = Some(p);
                        let d = self.to_design(p);
                        self.settings.handle(&Ev::Down(e.button(), d));
                    }
                    Mode::Paint => {
                        if p.0 < self.toolbar_width() {
                            self.click = Some(p);
                        } else if e.button() == 2 {
                            self.pan = Some((p, self.cam));
                        } else if e.button() == 1 || e.button() == 3 {
                            self.begin_stroke(e.button());
                            self.apply(true, 0);
                        }
                    }
                }
            }
            sdl::MOUSEBUTTONUP => {
                if self.mode == Mode::Settings {
                    let d = self.to_design((e.button_x(), e.button_y()));
                    self.settings.handle(&Ev::Up(e.button(), d));
                }
                if e.button() == 2 {
                    self.pan = None;
                } else if Some(e.button()) == self.stroke {
                    self.stroke = None;
                    // nothing changed: no undo step
                    if let (Some(m), Some(last)) = (&self.map, self.undo.last()) {
                        if m.g == *last {
                            self.undo.pop();
                        }
                    }
                }
            }
            sdl::MOUSEWHEEL if self.mode == Mode::Paint => {
                let vr = self.view_rect();
                if self.mouse.0 >= vr.x {
                    // zoom around the mouse
                    let before = self.tile_at(self.mouse);
                    let f = if e.wheel_y() > 0 { 1.25 } else { 0.8 };
                    self.zoom = (self.zoom * f).clamp(1.0, 64.0);
                    let after = self.tile_at(self.mouse);
                    self.cam.0 += (before.0 - after.0) as f64;
                    self.cam.1 += (before.1 - after.1) as f64;
                }
            }
            sdl::TEXTINPUT if self.mode == Mode::Settings => {
                self.settings.handle(&Ev::Text(e.text()));
            }
            sdl::KEYDOWN => {
                let k = e.key_sym();
                match self.mode {
                    Mode::Settings => {
                        if k == sdl::K_ESCAPE {
                            self.close_settings();
                        } else {
                            let acts = self.settings.handle(&Ev::Key(k));
                            if acts.contains(&Action::Resume) {
                                self.close_settings();
                            }
                        }
                    }
                    Mode::Choose => {
                        if k == sdl::K_ESCAPE {
                            return Some(Exit::Back);
                        }
                    }
                    Mode::Paint => match k {
                        K_Z if ctrl => self.undo(),
                        sdl::K_S if ctrl => self.pending = Some(Pending::Save),
                        sdl::K_ESCAPE => self.pending = Some(Pending::Exit),
                        K_LEFTBRACKET | sdl::K_MINUS => self.brush = (self.brush - 1).max(1),
                        K_RIGHTBRACKET | sdl::K_EQUALS => self.brush = (self.brush + 1).min(12),
                        sdl::K_HOME => {
                            if let Some(m) = &self.map {
                                self.cam = (m.w as f64 / 2.0, m.h as f64 / 2.0);
                            }
                            self.fit_zoom();
                        }
                        _ => {}
                    },
                }
            }
            _ => {}
        }
        None
    }
}

const K_Z: i32 = 122;
const K_LEFTBRACKET: i32 = 91;
const K_RIGHTBRACKET: i32 = 93;

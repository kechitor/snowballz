//! A very small widget toolkit for the menus (the port of ui.py), using the
//! theme in data/themes/snowballz/.
//!
//! Widgets are laid out in "design coordinates" (the 640x480 area the menu
//! was made for); `View` maps that area onto the screen and everything is
//! drawn at the final resolution, so text stays sharp.

use crate::gfx::{self, Font, Tex};
use crate::sdl::{self, Rect};

pub const THEME_DIR: &str = "data/themes/snowballz";
pub const TEXT_COLOR: [f32; 4] = [5.0 / 255.0, 5.0 / 255.0, 5.0 / 255.0, 1.0];
pub const INPUT_COLOR: [f32; 4] = [0.0, 0.0, 90.0 / 255.0, 1.0];

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub scale: f64,
    pub off_x: f64,
    pub off_y: f64,
}

impl View {
    pub fn to_screen(&self, r: &DRect) -> Rect {
        let left = (self.off_x + r.x as f64 * self.scale).round() as i32;
        let top = (self.off_y + r.y as f64 * self.scale).round() as i32;
        let right = (self.off_x + (r.x + r.w) as f64 * self.scale).round() as i32;
        let bottom = (self.off_y + (r.y + r.h) as f64 * self.scale).round() as i32;
        Rect::new(left, top, right - left, bottom - top)
    }
    pub fn px(&self, v: f64) -> i32 {
        (v * self.scale).round() as i32
    }
}

/// A rectangle in design coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct DRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl DRect {
    pub fn contains(&self, p: (i32, i32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

pub fn font(size: u32) -> Font {
    gfx::load_font(&format!("{}/TSCu_Comic.ttf", THEME_DIR), size)
        .or_else(|_| gfx::load_font("data/TSCu_Comic.ttf", size))
        .expect("the menu font is missing")
}

fn theme(name: &str) -> Tex {
    gfx::image_texture(&format!("{}/{}", THEME_DIR, name)).unwrap_or_else(|e| {
        eprintln!("{}", e);
        gfx::texture("data/noicon.png")
    })
}

/// Text drawn at the screen size matching `design_size`.
fn draw_text(v: &View, text: &str, design_size: u32, color: [f32; 4], x: i32, y: i32) {
    let px = ((design_size as f64 * v.scale).round() as u32).max(4);
    font(px).draw_screen(x, y, text, color);
}

fn text_size(v: &View, text: &str, design_size: u32) -> (i32, i32) {
    let px = ((design_size as f64 * v.scale).round() as u32).max(4);
    font(px).size_of(text)
}

/// The theme image stretched to `r` keeping its (scaled) borders.
pub fn slice9(v: &View, name: &str, r: Rect) {
    let t = theme(name);
    let (iw, ih) = (t.w, t.h);
    let (sbx, sby) = (6, 8);
    let bx = ((6.0 * v.scale).round() as i32).max(1);
    let by = ((8.0 * v.scale).round() as i32).max(1);
    let w = r.w.max(2 * bx + 1);
    let h = r.h.max(2 * by + 1);
    let xs = [(0, sbx, 0, bx), (sbx, iw - 2 * sbx, bx, w - 2 * bx), (iw - sbx, sbx, w - bx, bx)];
    let ys = [(0, sby, 0, by), (sby, ih - 2 * sby, by, h - 2 * by), (ih - sby, sby, h - by, by)];
    for &(sx, sw, dx, dw) in &xs {
        for &(sy, sh, dy, dh) in &ys {
            if sw > 0 && sh > 0 && dw > 0 && dh > 0 {
                gfx::blit(&t, Some(Rect::new(t.sx + sx, t.sy + sy, sw, sh)), Rect::new(r.x + dx, r.y + dy, dw, dh), 1.0);
            }
        }
    }
}

/// Events in design coordinates.
#[derive(Clone, Debug)]
pub enum Ev {
    Motion((i32, i32)),
    Down(u8, (i32, i32)),
    Up(u8, (i32, i32)),
    Wheel(i32),
    Key(i32),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Page(usize),
    Exit,
    Seek(i32),
    /// one more (+1) or one less (-1) snowballer in the loadout
    Loadout(i32),
    SinglePlay,
    Host,
    Join,
    /// a switch changed: (option name, new value)
    Option(String, String),
    /// a slider moved: (option name, percent, released: save it now)
    Level(String, u8, bool),
    NameChanged(String),
    /// the in-game menu: back to the game
    Resume,
    /// the main menu: open the map editor
    MapEditor,
}

pub struct Label {
    pub text: String,
    pub size: u32,
    pub color: [f32; 4],
    pub min_width: i32,
    pub rect: DRect,
    pub visible: bool,
}

impl Label {
    pub fn new(text: &str, size: u32, x: i32, y: i32) -> Label {
        let mut l = Label { text: String::new(), size, color: TEXT_COLOR, min_width: 0, rect: DRect { x, y, w: 0, h: 0 }, visible: true };
        l.set_text(text);
        l
    }
    pub fn set_text(&mut self, t: &str) {
        self.text = t.to_string();
        let (w, h) = font(self.size).size_of(&self.text);
        self.rect.w = w.max(self.min_width);
        self.rect.h = h;
    }
    fn draw(&self, v: &View) {
        let r = v.to_screen(&self.rect);
        draw_text(v, &self.text, self.size, self.color, r.x, r.y);
    }
}

pub struct ImageW {
    pub tex: Tex,
    pub rect: DRect,
}

impl ImageW {
    fn draw(&self, v: &View) {
        let r = v.to_screen(&self.rect);
        gfx::blit(&self.tex, None, r, 1.0);
    }
}

pub struct Button {
    pub text: String,
    pub action: Action,
    pub rect: DRect,
    pub min_width: i32,
    pub size: u32,
    pub hover: bool,
    pub down: bool,
    /// a switch: (option, values, labels, current index)
    pub switch: Option<(String, Vec<String>, Vec<String>, usize)>,
}

impl Button {
    pub fn new(text: &str, action: Action) -> Button {
        let mut b = Button { text: String::new(), action, rect: DRect::default(), min_width: 60, size: 20, hover: false, down: false, switch: None };
        b.set_text(text);
        b
    }

    pub fn switch(option: &str, value: &str, values: &[&str], labels: &[String]) -> Button {
        let idx = values.iter().position(|v| *v == value).unwrap_or(0);
        let mut b = Button::new(&labels[idx], Action::Exit);
        b.min_width = 150;
        b.size = 16;
        b.switch = Some((option.to_string(), values.iter().map(|s| s.to_string()).collect(), labels.to_vec(), idx));
        b.set_text(&labels[idx]);
        b
    }

    pub fn set_text(&mut self, t: &str) {
        self.text = t.to_string();
        let (w, h) = font(self.size).size_of(&self.text);
        self.rect.w = self.min_width.max(w + 20);
        self.rect.h = h + 10;
    }

    /// Show `value` (for switches changed from elsewhere, e.g. F11).
    pub fn set_value(&mut self, value: &str) {
        if let Some((_, values, labels, idx)) = &mut self.switch {
            if let Some(i) = values.iter().position(|v| v == value) {
                *idx = i;
                let t = labels[i].clone();
                self.set_text(&t);
            }
        }
    }

    fn clicked(&mut self) -> Action {
        if let Some((opt, values, labels, idx)) = &mut self.switch {
            *idx = (*idx + 1) % values.len();
            let (o, v, t) = (opt.clone(), values[*idx].clone(), labels[*idx].clone());
            self.set_text(&t);
            return Action::Option(o, v);
        }
        self.action.clone()
    }

    fn handle(&mut self, e: &Ev) -> (bool, Option<Action>) {
        match e {
            Ev::Motion(p) => self.hover = self.rect.contains(*p),
            Ev::Down(1, p) if self.rect.contains(*p) => {
                self.down = true;
                return (true, None);
            }
            Ev::Up(1, p) => {
                let was = self.down;
                self.down = false;
                if was && self.rect.contains(*p) {
                    return (true, Some(self.clicked()));
                }
            }
            _ => {}
        }
        (false, None)
    }

    fn draw(&self, v: &View) {
        let name = if self.down {
            "button_down.png"
        } else if self.hover {
            "button_hover.png"
        } else {
            "button.png"
        };
        let r = v.to_screen(&self.rect);
        slice9(v, name, r);
        let (tw, th) = text_size(v, &self.text, self.size);
        draw_text(v, &self.text, self.size, TEXT_COLOR, r.x + (r.w - tw) / 2, r.y + (r.h - th) / 2);
    }
}

pub struct Input {
    pub value: String,
    pub disallowed: Vec<char>,
    pub size: u32,
    pub hover: bool,
    pub focused: bool,
    pub rect: DRect,
    pub is_name: bool,
    /// the most characters allowed
    pub max_len: usize,
    /// what Enter does (besides leaving the box)
    pub submit: Option<Action>,
}

impl Input {
    pub fn new(value: &str, width: i32, size: u32) -> Input {
        let h = font(size).metrics().height + 10;
        Input {
            value: value.to_string(),
            disallowed: vec![' '],
            size,
            hover: false,
            focused: false,
            rect: DRect { x: 0, y: 0, w: width, h },
            is_name: false,
            max_len: 40,
            submit: None,
        }
    }

    fn insert(&mut self, text: &str) {
        for c in text.chars() {
            if !c.is_control() && !self.disallowed.contains(&c) && self.value.chars().count() < self.max_len {
                self.value.push(c);
            }
        }
    }

    fn handle(&mut self, e: &Ev) -> (bool, Option<Action>) {
        let old = self.value.clone();
        match e {
            Ev::Motion(p) => {
                self.hover = self.rect.contains(*p);
                return (false, None);
            }
            Ev::Down(1, p) => {
                self.focused = self.rect.contains(*p);
                return (self.focused, None);
            }
            Ev::Key(k) if self.focused => {
                let mods = sdl::mod_state();
                let paste = (*k == sdl::K_V && mods & (sdl::KMOD_CTRL | sdl::KMOD_GUI) != 0)
                    || (*k == sdl::K_INSERT && mods & sdl::KMOD_SHIFT != 0);
                let clear = *k == sdl::K_BACKSPACE && mods & (sdl::KMOD_CTRL | sdl::KMOD_GUI) != 0;
                match *k {
                    _ if paste => {
                        let t = sdl::clipboard();
                        self.insert(t.trim());
                    }
                    _ if clear => self.value.clear(),
                    sdl::K_BACKSPACE => {
                        self.value.pop();
                    }
                    sdl::K_RETURN | sdl::K_KP_ENTER => {
                        self.focused = false;
                        if let Some(a) = &self.submit {
                            return (true, Some(a.clone()));
                        }
                    }
                    sdl::K_ESCAPE | sdl::K_TAB => self.focused = false,
                    _ => {}
                }
            }
            Ev::Text(t) if self.focused => {
                let t = t.clone();
                self.insert(&t);
            }
            _ => return (false, None),
        }
        let act = if self.value != old && self.is_name { Some(Action::NameChanged(self.value.clone())) } else { None };
        (true, act)
    }

    fn draw(&self, v: &View) {
        let r = v.to_screen(&self.rect);
        let active = self.focused || self.hover;
        slice9(v, if active { "input_focused.png" } else { "input.png" }, r);
        let text = format!("{}{}", self.value, if self.focused { "_" } else { "" });
        if text.is_empty() {
            return;
        }
        let color = if active { TEXT_COLOR } else { INPUT_COLOR };
        let pad = v.px(6.0);
        let room = r.w - 2 * pad;
        let (tw, th) = text_size(v, &text, self.size);
        // show the end of a long text
        let shift = (tw - room).max(0);
        gfx::set_clip(Some(Rect::new(r.x + pad, r.y, room, r.h)));
        draw_text(v, &text, self.size, color, r.x + pad - shift, r.y + v.px(5.0));
        let _ = th;
        gfx::set_clip(None);
    }
}

/// A 0-100 % slider (the volume settings).
pub struct Slider {
    pub option: String,
    pub value: u8,
    pub rect: DRect,
    pub hover: bool,
    pub dragging: bool,
}

impl Slider {
    /// Room for the "100%" text on the right.
    const TEXT_W: i32 = 44;
    const KNOB_W: i32 = 12;

    pub fn new(option: &str, value: u8, width: i32) -> Slider {
        Slider { option: option.to_string(), value: value.min(100), rect: DRect { x: 0, y: 0, w: width, h: 28 }, hover: false, dragging: false }
    }

    /// The track's left end and length (design coordinates).
    fn track(&self) -> (i32, i32) {
        let left = self.rect.x + Self::KNOB_W / 2;
        (left, self.rect.w - Self::TEXT_W - Self::KNOB_W)
    }

    fn value_at(&self, x: i32) -> u8 {
        let (left, len) = self.track();
        (((x - left) as f64 / len.max(1) as f64 * 100.0).round().clamp(0.0, 100.0)) as u8
    }

    fn set(&mut self, v: u8, released: bool) -> (bool, Option<Action>) {
        let changed = v != self.value;
        self.value = v;
        if changed || released {
            (true, Some(Action::Level(self.option.clone(), v, released)))
        } else {
            (true, None)
        }
    }

    fn handle(&mut self, e: &Ev) -> (bool, Option<Action>) {
        let slider_area = DRect { w: self.rect.w - Self::TEXT_W + 4, ..self.rect };
        match e {
            Ev::Motion(p) => {
                self.hover = self.rect.contains(*p);
                if self.dragging {
                    return self.set(self.value_at(p.0), false);
                }
            }
            Ev::Down(1, p) if slider_area.contains(*p) => {
                self.dragging = true;
                return self.set(self.value_at(p.0), false);
            }
            Ev::Up(1, p) if self.dragging => {
                self.dragging = false;
                let (_, a) = self.set(self.value_at(p.0), true);
                return (false, a);
            }
            Ev::Wheel(y) if self.hover => {
                let v = (self.value as i32 + 5 * y).clamp(0, 100) as u8;
                return self.set(v, true);
            }
            _ => {}
        }
        (false, None)
    }

    fn draw(&self, v: &View) {
        let (left, len) = self.track();
        let cy = self.rect.y + self.rect.h / 2;
        // the groove, filled up to the value
        let groove = v.to_screen(&DRect { x: left - 2, y: cy - 5, w: len + 4, h: 10 });
        let b = v.px(1.0).max(1);
        gfx::fill_screen_rect(groove, (20, 30, 80, 255));
        gfx::fill_screen_rect(Rect::new(groove.x + b, groove.y + b, groove.w - 2 * b, groove.h - 2 * b), (70, 80, 130, 255));
        let pos = left + (len as f64 * self.value as f64 / 100.0).round() as i32;
        let fill = v.to_screen(&DRect { x: left - 2, y: cy - 5, w: pos - left + 2, h: 10 });
        if fill.w > 2 * b {
            gfx::fill_screen_rect(Rect::new(fill.x + b, fill.y + b, fill.w - 2 * b, fill.h - 2 * b), (110, 170, 255, 255));
        }
        // the knob
        let knob = v.to_screen(&DRect { x: pos - Self::KNOB_W / 2, y: self.rect.y + 1, w: Self::KNOB_W, h: self.rect.h - 2 });
        let name = if self.dragging {
            "button_down.png"
        } else if self.hover {
            "button_hover.png"
        } else {
            "button.png"
        };
        slice9(v, name, knob);
        let text = format!("{}%", self.value);
        let r = v.to_screen(&self.rect);
        let (tw, th) = text_size(v, &text, 16);
        draw_text(v, &text, 16, TEXT_COLOR, r.x + r.w - tw, r.y + (r.h - th) / 2);
    }
}

pub struct SelectBox {
    pub items: Vec<(String, String)>,
    pub selected: Option<usize>,
    pub scroll: usize,
    pub hover: Option<usize>,
    pub rect: DRect,
}

const ROW: i32 = 22;

impl SelectBox {
    fn row_at(&self, p: (i32, i32)) -> Option<usize> {
        if !self.rect.contains(p) {
            return None;
        }
        let i = (p.1 - self.rect.y - 4).div_euclid(ROW) + self.scroll as i32;
        if i >= 0 && (i as usize) < self.items.len() {
            Some(i as usize)
        } else {
            None
        }
    }

    pub fn selected_value(&self) -> Option<String> {
        self.selected.map(|i| self.items[i].1.clone())
    }

    fn handle(&mut self, e: &Ev) -> (bool, Option<Action>) {
        match e {
            Ev::Motion(p) => self.hover = self.row_at(*p),
            Ev::Down(1, p) => {
                if let Some(i) = self.row_at(*p) {
                    self.selected = Some(i);
                    return (true, None);
                }
            }
            Ev::Wheel(y) => {
                let visible = ((self.rect.h - 8) / ROW) as i32;
                let max = (self.items.len() as i32 - visible).max(0);
                self.scroll = (self.scroll as i32 - y).clamp(0, max) as usize;
            }
            _ => {}
        }
        (false, None)
    }

    fn draw(&self, v: &View) {
        let r = v.to_screen(&self.rect);
        slice9(v, "input.png", r);
        let pad = v.px(4.0);
        gfx::set_clip(Some(Rect::new(r.x + pad / 2, r.y + pad / 2, r.w - pad, r.h - pad)));
        let row_h = ROW as f64 * v.scale;
        let mut y = (r.y + pad) as f64;
        for i in self.scroll..self.items.len() {
            let row = Rect::new(r.x + pad, y.round() as i32, r.w - 2 * pad, (row_h - v.scale).round() as i32);
            let a = if Some(i) == self.selected {
                200
            } else if Some(i) == self.hover {
                100
            } else {
                50
            };
            gfx::fill_screen_rect(row, (150, 150, 255, a));
            let (_, th) = text_size(v, &self.items[i].0, 16);
            draw_text(v, &self.items[i].0, 16, TEXT_COLOR, row.x + v.px(9.0), row.y + (row.h - th) / 2);
            y += row_h;
        }
        gfx::set_clip(None);
    }
}

pub enum Widget {
    Label(Label),
    Image(ImageW),
    Button(Button),
    Input(Input),
    Select(SelectBox),
    Slider(Slider),
    Panel(Panel),
}

impl Widget {
    pub fn rect(&self) -> DRect {
        match self {
            Widget::Label(w) => w.rect,
            Widget::Image(w) => w.rect,
            Widget::Button(w) => w.rect,
            Widget::Input(w) => w.rect,
            Widget::Select(w) => w.rect,
            Widget::Slider(w) => w.rect,
            Widget::Panel(w) => w.rect,
        }
    }

    pub fn set_pos(&mut self, x: i32, y: i32) {
        let r = match self {
            Widget::Label(w) => &mut w.rect,
            Widget::Image(w) => &mut w.rect,
            Widget::Button(w) => &mut w.rect,
            Widget::Input(w) => &mut w.rect,
            Widget::Select(w) => &mut w.rect,
            Widget::Slider(w) => &mut w.rect,
            Widget::Panel(w) => &mut w.rect,
        };
        r.x = x;
        r.y = y;
    }

    pub fn handle(&mut self, e: &Ev) -> (bool, Option<Action>) {
        match self {
            Widget::Button(w) => w.handle(e),
            Widget::Input(w) => w.handle(e),
            Widget::Select(w) => w.handle(e),
            Widget::Slider(w) => w.handle(e),
            Widget::Panel(p) => {
                for row in p.rows.iter_mut() {
                    for w in row.iter_mut() {
                        let r = w.handle(e);
                        if r.0 || r.1.is_some() {
                            return r;
                        }
                    }
                }
                (false, None)
            }
            _ => (false, None),
        }
    }

    pub fn draw(&self, v: &View) {
        match self {
            Widget::Label(w) => {
                if w.visible {
                    w.draw(v)
                }
            }
            Widget::Image(w) => w.draw(v),
            Widget::Button(w) => w.draw(v),
            Widget::Input(w) => w.draw(v),
            Widget::Select(w) => w.draw(v),
            Widget::Slider(w) => w.draw(v),
            Widget::Panel(p) => p.draw(v),
        }
    }
}

/// Rows of (label, control) on a tiled background, centred on the page.
pub struct Panel {
    pub rows: Vec<Vec<Widget>>,
    pub spacing: i32,
    pub padding: i32,
    pub top: i32,
    pub rect: DRect,
}

impl Panel {
    pub fn new(top: i32, spacing: i32, padding: i32) -> Panel {
        Panel { rows: Vec::new(), spacing, padding, top, rect: DRect::default() }
    }

    pub fn add_row(&mut self, row: Vec<Widget>) {
        self.rows.push(row);
        self.layout();
    }

    pub fn layout(&mut self) {
        let gap = 20;
        let width = self
            .rows
            .iter()
            .map(|r| r.iter().map(|e| e.rect().w).sum::<i32>() + gap * (r.len() as i32 - 1))
            .max()
            .unwrap_or(0)
            + 2 * self.padding;
        let height = self.rows.iter().map(|r| r.iter().map(|e| e.rect().h).max().unwrap_or(0)).sum::<i32>()
            + self.spacing * (self.rows.len() as i32 - 1)
            + 2 * self.padding;
        let x0 = (640 - width) / 2;
        self.rect = DRect { x: x0, y: self.top, w: width, h: height };
        let mut y = self.top + self.padding;
        for row in self.rows.iter_mut() {
            let mut x = x0 + self.padding;
            let rh = row.iter().map(|e| e.rect().h).max().unwrap_or(0);
            for e in row.iter_mut() {
                let h = e.rect().h;
                e.set_pos(x, y + (rh - h) / 2);
                x += e.rect().w + gap;
            }
            y += rh + self.spacing;
        }
    }

    fn draw(&self, v: &View) {
        panel_bg(v, v.to_screen(&self.rect));
        for row in &self.rows {
            for e in row {
                e.draw(v);
            }
        }
    }
}

/// The panels' background (the theme's bg.png tiled over a screen
/// rectangle).
pub fn panel_bg(v: &View, r: Rect) {
    let bg = theme("bg.png");
    let tw = ((bg.w as f64 * v.scale).round() as i32).max(1);
    let th = ((bg.h as f64 * v.scale).round() as i32).max(1);
    gfx::set_clip(Some(r));
    let mut yy = r.y;
    while yy < r.y + r.h {
        let mut xx = r.x;
        while xx < r.x + r.w {
            gfx::blit(&bg, None, Rect::new(xx, yy, tw, th), 1.0);
            xx += tw;
        }
        yy += th;
    }
    gfx::set_clip(None);
}

/// A list of widgets drawn and updated together.
#[derive(Default)]
pub struct Page {
    pub widgets: Vec<Widget>,
}

impl Page {
    pub fn add(&mut self, w: Widget) -> usize {
        self.widgets.push(w);
        self.widgets.len() - 1
    }

    /// Returns the actions of clicked widgets.
    pub fn handle(&mut self, e: &Ev) -> Vec<Action> {
        let mut out = Vec::new();
        // mouse up reaches every widget so pressed buttons release
        for w in self.widgets.iter_mut().rev() {
            let (used, act) = w.handle(e);
            if let Some(a) = act {
                out.push(a);
            }
            if used && !matches!(e, Ev::Up(..)) {
                break;
            }
        }
        out
    }

    pub fn draw(&self, v: &View) {
        for w in &self.widgets {
            w.draw(v);
        }
    }
}

/// Lay widgets out in a row centred at `center_x`, with their bottoms at
/// `bottom` or centred at `center_y`.
pub fn hbox(ws: &mut [&mut Widget], center_x: i32, bottom: Option<i32>, center_y: Option<i32>, spacing: i32) {
    let total: i32 = ws.iter().map(|w| w.rect().w).sum::<i32>() + spacing * (ws.len() as i32 - 1);
    let mut x = center_x - total / 2;
    for w in ws.iter_mut() {
        let r = w.rect();
        let y = match (bottom, center_y) {
            (Some(b), _) => b - r.h,
            (None, Some(c)) => c - r.h / 2,
            _ => r.y,
        };
        w.set_pos(x, y);
        x += r.w + spacing;
    }
}

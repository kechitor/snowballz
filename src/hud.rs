//! The selection panel (selectctrl.py) and the minimap (minimap.py).

use crate::gfx;
use crate::world::*;

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

pub fn panel_visible(w: &World) -> bool {
    !w.selected.is_empty()
}

/// Draw the panel (ui mode).
pub fn draw_panel(w: &World) {
    if !panel_visible(w) {
        return;
    }
    let (workers, snowballers) = selected_counts(w);
    let vw = w.view.w as f64;
    let pc = w.me_color();
    gfx::texture("data/selectctrl.png").draw(vw - 64.0, 0.0, [1.0; 4], 1.0);
    let t = w.tex.penguins["walk"].get(10, (0, 1));
    let x = vw - 64.0 + 16.0;
    let black = [0.0, 0.0, 0.0, 1.0];
    if snowballers > 0 {
        let to = w.tex.penguins["snowballerwalk"].get(10, (0, 1));
        t.draw(x, 111.0, [1.0; 4], 1.0);
        to.draw(x, 111.0, [pc[0], pc[1], pc[2], 1.0], 1.0);
        w.chat_font.render(x + 8.0, 111.0 - 19.0, &snowballers.to_string(), black);
    }
    if workers > 0 {
        let to = w.tex.penguins["workerwalk"].get(10, (0, 1));
        t.draw(x, 58.0, [1.0; 4], 1.0);
        to.draw(x, 58.0, [pc[0], pc[1], pc[2], 1.0], 1.0);
        w.chat_font.render(x + 8.0, 58.0 - 19.0, &workers.to_string(), black);
    }
    gfx::texture("data/marker_leave.png").draw(vw - 64.0 + 16.0, 2.0, [pc[0], pc[1], pc[2], 1.0], 1.0);
}

/// Is the (screen) position over the panel?
pub fn in_panel(w: &World, pos: (i32, i32)) -> bool {
    panel_visible(w) && pos.0 > w.view.w - 64 && pos.1 > w.view.h - 160
}

pub enum PanelClick {
    Snowballers,
    Workers,
    Disband,
    Nothing,
}

pub fn panel_click(w: &World, pos: (i32, i32)) -> PanelClick {
    let (workers, snowballers) = selected_counts(w);
    let y = pos.1;
    if y < w.view.h - 160 + 64 {
        if snowballers == 0 {
            return PanelClick::Nothing;
        }
        PanelClick::Snowballers
    } else if y < w.view.h - 160 + 128 {
        if workers == 0 {
            return PanelClick::Nothing;
        }
        PanelClick::Workers
    } else {
        PanelClick::Disband
    }
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

// -- minimap ------------------------------------------------------------------

pub struct Minimap {
    pub scale: f64,
    pub draw_x: f64,
    pub draw_y: f64,
}

impl Minimap {
    pub fn new(w: &World) -> Minimap {
        let (vw, vh) = (w.view.w as f64, w.view.h as f64);
        let (mw, mh) = (w.size.0 as f64, w.size.1 as f64);
        let scale = (vh / mw).min(vw / mh);
        Minimap { scale, draw_x: (-(scale * mw - vw) / 2.0).floor(), draw_y: ((scale * mh - vh) / 2.0).floor() + vh }
    }

    pub fn draw(&self, w: &mut World) {
        gfx::push();
        gfx::translate(self.draw_x, self.draw_y);
        gfx::scale(self.scale, self.scale);
        gfx::fill_rect(0.0, 0.0, w.size.0 as f64, -(w.size.1 as f64), Some([0.0, 0.0, 0.0, 0.3]));
        for b in &w.buildings {
            if let Some(p) = b.player {
                let c = w.glcolor(Some(p));
                gfx::draw_points(&[(b.x as f64, -(b.y as f64))], [c[0], c[1], c[2], 1.0], 4);
            }
        }
        for ui in 0..w.units.len() {
            let c = w.glcolor(w.igloo_player(ui));
            let u = &w.units[ui];
            gfx::draw_points(&[(u.x as f64, -(u.y as f64))], [c[0], c[1], c[2], 1.0], 2);
        }
        for d in w.dmos.iter_mut() {
            if d.hidden {
                continue;
            }
            if let Some(img) = d.minimapimage.clone() {
                d.doanimate();
                let t = gfx::texture(&img);
                t.draw(d.position.0 as f64, -(d.position.1 as f64), [1.0, 1.0, 1.0, d.alpha as f32], 1.0);
            }
        }
        gfx::translate(w.view.x / 32.0, -w.view.y / 32.0);
        gfx::fill_rect(0.0, 0.0, w.view.scale_w() as f64 / 32.0, -(w.view.scale_h() as f64 / 32.0), Some([0.0, 0.3, 0.3, 0.3]));
        gfx::pop();
    }

    /// Screen position -> top left of the view, in world pixels.
    pub fn pos_to_mappos(&self, w: &World, cx: f64, cy: f64) -> (f64, f64) {
        let cx = cx - ((self.scale * w.view.scale_w() as f64 / 32.0) / 2.0).floor();
        let cy = cy - ((self.scale * w.view.scale_h() as f64 / 32.0) / 2.0).floor();
        let px = cx - self.draw_x;
        let py = cy - self.draw_y + self.scale * w.size.1 as f64;
        ((px * 32.0 / self.scale).floor(), (py * 32.0 / self.scale).floor())
    }
}

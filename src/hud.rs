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
    // how to change class
    let hint = format!("X: change class ({} fish each)", crate::unit::SWITCH_FISH);
    let (tw, th) = w.stat_font.size_of(&hint);
    let (tw, th) = (tw as f64, th as f64);
    let hx = vw - 64.0 - tw - 14.0;
    gfx::fill_rect(hx - 6.0, 4.0, tw + 12.0, th + 6.0, Some([0.07, 0.09, 0.16, 0.6]));
    w.stat_font.render(hx, 7.0, &hint, [0.9, 0.95, 1.0, 1.0]);
    // and (workers) walls
    if workers > 0 {
        let hint = format!("B: snow wall   U: ice wall ({} crystal an {}-tile section)", crate::wall::ICE_CRYSTAL, crate::wall::SECTION_TILES);
        let (tw, _) = w.stat_font.size_of(&hint);
        let tw = tw as f64;
        let hx = vw - 64.0 - tw - 14.0;
        let y = 4.0 + th + 10.0;
        gfx::fill_rect(hx - 6.0, y, tw + 12.0, th + 6.0, Some([0.07, 0.09, 0.16, 0.6]));
        w.stat_font.render(hx, y + 3.0, &hint, [0.9, 0.95, 1.0, 1.0]);
    }
}

/// Planning a wall: what to do (top middle).
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
    let (tw, th) = w.stat_font.size_of(&text);
    let (sw, _) = w.stat_font.size_of(sub);
    let (tw, th, sw) = (tw as f64, th as f64, sw as f64);
    // (ui mode counts y from the bottom: just under the power-up countdown)
    let bh = th * 2.0 + 14.0;
    let y = w.view.h as f64 - 52.0 - bh;
    let bw = tw.max(sw) + 20.0;
    gfx::fill_rect(vw / 2.0 - bw / 2.0, y, bw, bh, Some([0.07, 0.09, 0.16, 0.75]));
    w.stat_font.render(vw / 2.0 - tw / 2.0, y + 8.0 + th, &text, [1.0; 4]);
    w.stat_font.render(vw / 2.0 - sw / 2.0, y + 5.0, sub, [0.75, 0.82, 0.95, 1.0]);
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

// --- squads -----------------------------------------------------------

const SQUAD_W: f64 = 54.0;
const SQUAD_H: f64 = 40.0;
const SQUAD_GAP: f64 = 6.0;
const SQUAD_MARGIN: f64 = 10.0;

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
    let sh = w.view.h as f64;
    for (n, x, top) in squad_buttons(w) {
        let members = crate::ctrl::squad_members(w, n);
        let lit = !w.selected.is_empty() && w.selected.len() == members.len() && members.iter().all(|u| w.selected.contains(u));
        // (drawing goes up from the bottom of the screen)
        let y = sh - top - SQUAD_H;
        let bg = if lit { [0.25, 0.35, 0.75, 0.9] } else { [0.07, 0.09, 0.16, 0.75] };
        gfx::fill_rect(x, y, SQUAD_W, SQUAD_H, Some(bg));
        if lit {
            gfx::fill_rect(x, y + SQUAD_H - 3.0, SQUAD_W, 3.0, Some([1.0, 1.0, 1.0, 0.9]));
        }
        let num = n.to_string();
        let (nw, nh) = w.chat_font.size_of(&num);
        w.chat_font.render(x + 8.0, y + (SQUAD_H - nh as f64) / 2.0, &num, [1.0, 1.0, 1.0, 1.0]);
        let count = members.len().to_string();
        let (cw, _) = w.stat_font.size_of(&count);
        let _ = nw;
        w.stat_font.render(x + SQUAD_W - cw as f64 - 7.0, y + 6.0, &count, [0.75, 0.82, 0.95, 1.0]);
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
    let font = w.chat_font;
    let (sw, sh) = (w.view.w as f64, w.view.h as f64);
    let title = format!("Crystal: first to {}", target);
    let (title_w, lh) = font.size_of(&title);
    let lh = lh as f64 + 6.0;
    let num_w = rows.iter().map(|r| font.size_of(&format!("{}/{}", r.3, r.3)).0).max().unwrap_or(0) as f64;
    let name_w = rows.iter().map(|r| font.size_of(&r.0).0).max().unwrap_or(0) as f64;
    let swatches = rows.iter().map(|r| r.1.len()).max().unwrap_or(1) as f64;
    let sw_w = swatches * 16.0 - 2.0;
    let bar_w = 60.0;
    let width = (sw_w + 8.0 + name_w + 10.0 + bar_w + 10.0 + num_w).max(title_w as f64) + 24.0;
    let height = lh * (rows.len() as f64 + 1.0) + 14.0;
    let x0 = sw - width - 10.0;
    let top = 10.0;
    gfx::fill_rect(x0, sh - top - height, width, height, Some([0.07, 0.09, 0.16, 0.72]));
    let mut y = sh - top - 8.0 - lh + 4.0;
    font.render(x0 + 12.0, y, &title, [1.0, 0.85, 0.5, 1.0]);
    for (name, colors, n, goal, me, alive) in rows {
        y -= lh;
        let a = if alive { 1.0 } else { 0.45 };
        let mut x = x0 + 12.0;
        for (i, c) in colors.iter().enumerate() {
            gfx::fill_rect(x + i as f64 * 16.0, y + 3.0, 14.0, 14.0, Some([c[0], c[1], c[2], a]));
        }
        x += sw_w + 8.0;
        let text_c = if me { [1.0, 1.0, 0.75, a] } else { [0.9, 0.94, 1.0, a] };
        font.render(x, y, &name, text_c);
        x += name_w + 10.0;
        let f = (n as f64 / goal as f64).clamp(0.0, 1.0);
        let c = colors[0];
        gfx::fill_rect(x, y + 7.0, bar_w, 6.0, Some([1.0, 1.0, 1.0, 0.18 * a]));
        gfx::fill_rect(x, y + 7.0, bar_w * f, 6.0, Some([c[0], c[1], c[2], a]));
        x += bar_w + 10.0;
        font.render(x, y, &format!("{}/{}", n, goal), text_c);
    }
}

// --- alliances --------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AllyAction {
    Offer(Pid),
    Accept(Pid),
    Reject(Pid),
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
        let sh = w.view.h as f64;
        let y = sh - self.y - self.h;
        let bg = match (self.on, self.action) {
            (false, _) => [0.3, 0.32, 0.38, 0.8],
            (_, AllyAction::Reject(_)) => [0.55, 0.2, 0.2, 0.95],
            (_, AllyAction::Accept(_)) => [0.18, 0.5, 0.28, 0.95],
            _ => [0.25, 0.35, 0.75, 0.95],
        };
        gfx::fill_rect(self.x, y, self.w, self.h, Some(bg));
        let (tw, th) = w.stat_font.size_of(&self.label);
        let c = if self.on { [1.0; 4] } else { [0.75, 0.75, 0.8, 1.0] };
        w.stat_font.render(self.x + (self.w - tw as f64) / 2.0, y + (self.h - th as f64) / 2.0, &self.label, c);
    }
}

const ROW_H: f64 = 34.0;
const PANEL_W: f64 = 470.0;
const PANEL_X: f64 = 16.0;
const PANEL_TOP: f64 = 110.0;
const POPUP_TOP: f64 = 58.0;

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

/// What a click at this screen position does (None: not on alliance UI).
pub fn ally_click(w: &World, pos: (i32, i32)) -> Option<AllyAction> {
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
    let sh = w.view.h as f64;
    let font = w.chat_font;
    let now = w.now();
    // the hint, above the squad buttons
    if !w.chat.chatting {
        let hint = if w.diplomacy_open { "P: close alliances" } else { "P: alliances" };
        let (tw, th) = w.stat_font.size_of(hint);
        let y = 10.0 + 40.0 + 8.0;
        gfx::fill_rect(10.0, y, tw as f64 + 12.0, th as f64 + 6.0, Some([0.07, 0.09, 0.16, 0.6]));
        w.stat_font.render(16.0, y + 3.0, hint, [0.9, 0.95, 1.0, 1.0]);
    }
    if let Some((x, y, width, h, text)) = popup_rect(w) {
        let yy = sh - y - h;
        gfx::fill_rect(x, yy, width, h, Some([0.07, 0.09, 0.16, 0.88]));
        gfx::fill_rect(x, yy, 4.0, h, Some([1.0, 0.85, 0.4, 1.0]));
        let (_, th) = font.size_of(&text);
        font.render(x + 14.0, yy + (h - th as f64) / 2.0, &text, [1.0, 0.95, 0.75, 1.0]);
    }
    if w.diplomacy_open {
        let h = panel_height(w);
        gfx::fill_rect(PANEL_X, sh - PANEL_TOP - h, PANEL_W, h, Some([0.07, 0.09, 0.16, 0.88]));
        let mut y = sh - PANEL_TOP - ROW_H + 6.0;
        font.render(PANEL_X + 14.0, y, "Alliances", [1.0, 0.85, 0.5, 1.0]);
        let sub = format!("{} fish each side; you have {}. Allies don't fight and win together.", crate::alliance::ALLIANCE_FISH, crate::alliance::fish_of(w, me));
        w.stat_font.render(PANEL_X + 14.0, y - 18.0, &sub, [0.75, 0.82, 0.95, 1.0]);
        y -= ROW_H * 1.5 - 6.0;
        for p in panel_players(w) {
            let pl = &w.players[&p];
            let alive = crate::alliance::alive(w, p);
            let a = if alive { 1.0 } else { 0.45 };
            let c = pl.glcolor;
            gfx::fill_rect(PANEL_X + 14.0, y + 2.0, 14.0, 14.0, Some([c[0], c[1], c[2], a]));
            font.render(PANEL_X + 36.0, y - 2.0, &pl.name, [0.9, 0.94, 1.0, a]);
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
                w.stat_font.render(PANEL_X + PANEL_W - 190.0, y, &t, [col[0], col[1], col[2], 1.0]);
            }
            y -= ROW_H;
        }
    }
    for b in ally_buttons(w) {
        b.draw(w);
    }
}

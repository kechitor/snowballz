//! Drawing a game frame (display.py) - which also runs the clock of the day
//! and moves the penguins, like the original.

use crate::backgrounds;
use crate::clock;
use crate::game::Session;
use crate::gfx::{self, Tex};
use crate::hud;
use crate::igloo;
use crate::settings;
use crate::snowball;
use crate::unit;
use crate::world::*;

/// Draw a penguin (into the draw buffer; the warmth bar and group number
/// straight away).
fn draw_unit(w: &mut World, ui: usize) {
    let t = gfx::time();
    if w.units[ui].direction == (0, 0) {
        w.units[ui].direction = (0, 1);
    }
    let (x, y) = w.units[ui].sprite_xy(t);
    w.units[ui].draw_x = x;
    w.units[ui].draw_y = y;
    let color = w.glcolor(w.igloo_player(ui));
    // hit (its warmth went down): a quick blue flash
    let now = w.now();
    {
        let u = &mut w.units[ui];
        // (not when it was only capped at a lower most warmth: a class
        // change, a lost defense power-up)
        if u.warmth < u.drawn_warmth && u.warmth < u.max_warmth {
            u.hit_at = Some(now);
        }
        u.drawn_warmth = u.warmth;
    }
    let flash = match w.units[ui].hit_at {
        // (full strength for the first part, then fading)
        Some(t) if now.wrapping_sub(t) < HIT_FLASH_MS => ((1.0 - now.wrapping_sub(t) as f32 / HIT_FLASH_MS as f32) / 0.6).min(1.0),
        _ => 0.0,
    };
    let tint = |c: [f32; 4]| -> [f32; 4] {
        let b = [0.12, 0.38, 1.0];
        [c[0] * (1.0 - flash * (1.0 - b[0])), c[1] * (1.0 - flash * (1.0 - b[1])), c[2] * (1.0 - flash * (1.0 - b[2])), c[3]]
    };

    if w.view.zoom < 0.5 {
        // so far out: just a box
        let alpha = (1.0 - (w.view.zoom - 0.2) / 0.2).min(1.0) as f32;
        gfx::fill_rect(x, y, 32.0, 32.0, Some([color[0], color[1], color[2], alpha]));
        if alpha >= 1.0 {
            return;
        }
    }
    if w.units[ui].current_frame > 9 {
        w.units[ui].current_frame = 0;
    }
    let u = &w.units[ui];
    let frame = u.current_frame.clamp(0, 9) as usize;
    let dir = u.direction;
    let p = &w.tex.penguins;
    let pick = |a: &str, b: &str, f: usize| -> (Option<Tex>, Option<Tex>) { (p.get(a).map(|x| x.get(f, dir)), p.get(b).map(|x| x.get(f, dir))) };
    let (tex, outfit) = match u.state {
        State::Frozen => (Some(w.tex.frozen), None),
        State::Unloading => {
            let h = u.holding.unwrap_or(ResKind::Fish).name();
            pick(&format!("unload{}", h), &format!("workerunload{}", h), frame)
        }
        State::Throwing | State::GettingSnowball | State::Gathering => {
            let s = match u.state {
                State::Throwing => "throwing",
                State::GettingSnowball => "getting_snowball",
                _ => "gathering",
            };
            pick(s, &format!("{}{}", u.kind.name(), s), frame)
        }
        _ if u.kind == UnitKind::Worker && u.holding.is_some() => {
            let h = u.holding.unwrap().name();
            pick(&format!("walk{}", h), &format!("workerwalk{}", h), frame)
        }
        _ => {
            let f = if u.moving { frame } else { 10 };
            pick("walk", &format!("{}walk", u.kind.name()), f)
        }
    };
    if u.selected {
        w.draw_buffer.push(Rendered::tex(w.tex.selected, x, y, 1, [1.0; 4]));
    }
    let key = (1, y + 32.0, -x);
    if let Some(t) = tex {
        w.draw_buffer.push(Rendered { key, item: DrawItem::Tex { tex: t, x, y, color: tint([1.0; 4]) } });
    }
    if let Some(o) = outfit {
        w.draw_buffer.push(Rendered { key, item: DrawItem::Tex { tex: o, x, y, color: tint([color[0], color[1], color[2], 1.0]) } });
    }
    if u.state == State::Frozen {
        // a little flag in its team's colour stuck in the snow beside the ice
        // (drawn right after the ice block, in front of it)
        let fk = (1, y + 32.0 + 0.01, -x);
        let item = DrawItem::Tex { tex: w.tex.frozen_flag, x: x + 22.0, y: y - 6.0, color: [color[0], color[1], color[2], 1.0] };
        w.draw_buffer.push(Rendered { key: fk, item });
    }
    for e in &u.emblems {
        let et = gfx::texture(&e.image_name);
        w.draw_buffer.push(Rendered::tex(et, x + e.offset.0 as f64, y + e.offset.1 as f64, 5, [1.0, 1.0, 1.0, e.alpha as f32]));
    }
    // warmth bar
    if u.state != State::Frozen && u.warmth < u.max_warmth {
        let m = u.max_warmth as f64;
        let wv = u.warmth as f64;
        let b = ((wv / m * 2.0) - 2.0).abs().clamp(0.0, 1.0) as f32;
        let g = (wv / m * 2.0).clamp(0.0, 1.0) as f32;
        let h = wv * 20.0 / m;
        gfx::fill_rect(x + 32.0, y + 28.0 - h, 2.0, h, Some([0.0, g, b, 1.0]));
    }
    if let Some(g) = u.group {
        // its squad number on a dark tag (white alone vanished on the snow)
        let t = g.to_string();
        let (tw, th) = w.stat_font.size_of(&t);
        gfx::fill_rect(x - 3.0, y - 2.0, tw as f64 + 6.0, th as f64 + 2.0, Some([0.07, 0.09, 0.16, 0.85]));
        w.stat_font.render(x, y, &t, [1.0; 4]);
    }
    // on its way home to change class (mine only)
    if u.switch_pending && w.igloo_player(ui) == w.me && !matches!(u.state, State::InBuilding | State::Entering) {
        let to = match u.kind {
            UnitKind::Worker => "to snowballer",
            UnitKind::Snowballer => "to worker",
        };
        let (tw, _) = w.stat_font.size_of(to);
        w.stat_font.render(x + 16.0 - tw as f64 / 2.0, y - 12.0, to, [0.1, 0.12, 0.25, 1.0]);
    }
}

fn draw_building(w: &mut World, bi: usize) {
    let b = &w.buildings[bi];
    let x = (b.x * 32) as f64;
    let y = (b.y * 32) as f64 - w.tiles[b.tile].elevation * ELEV_MULT;
    w.draw_buffer.push(Rendered::tex(w.tex.igloo, x, y, 3, [1.0; 4]));
    let Some(owner) = b.player else { return };
    let marker_y = y + ((IGLOO_SIZE - 1) * 32) as f64;
    let mx = x + (IGLOO_SIZE * 32 - 32) as f64;
    if b.jeopardy {
        w.draw_buffer.push(Rendered::tex(w.tex.jeopardy_marker, mx, marker_y, 4, [1.0; 4]));
        let left = (b.jeopardy_count as i64 + TAKEOVER_TIME as i64 - w.now() as i64).div_euclid(1000);
        let c = w.glcolor(b.player_taking_over);
        w.stat_font.render(x, y, &format!("Takeover: {}", left), [c[0], c[1], c[2], 1.0]);
    } else {
        let c = w.glcolor(Some(owner));
        w.draw_buffer.push(Rendered::tex(w.tex.marker, mx, marker_y, 4, [c[0], c[1], c[2], 1.0]));
    }
    if Some(owner) == w.me {
        let mut s = Vec::new();
        if b.storage[0] != 0 {
            s.push(format!("fish: {}", b.storage[0]));
        }
        if b.storage[1] != 0 {
            s.push(format!("crystal: {}", b.storage[1]));
        }
        w.stat_font.render(x, marker_y + 25.0, &s.join("  "), [0.0, 0.0, 0.0, 1.0]);
    }
}

pub fn draw(s: &mut Session, w: &mut World, dt: u32) {
    crate::alerts::tick(w);
    let now = clock::now();
    gfx::set_time(now);
    let (vw, vh) = (w.view.scale_w() as f64, w.view.scale_h() as f64);

    // the time of day: half a day takes 2 minutes; resources come back at
    // midnight, frozen penguins thaw at noon
    // (only the server changes the world; clients hear about it)
    if now as f64 > s.last_time_tick as f64 + TIME_TICK_MS {
        s.last_time_tick = now;
        s.time += 1;
        if s.time == 88 {
            s.time = 0;
            if w.is_server() {
                for r in 0..w.resources.len() {
                    let q = w.rng.randint(5, 10);
                    w.set_quantity(r, q);
                }
            }
        }
        if s.time == 44 && w.is_server() {
            for ui in 0..w.units.len() {
                if w.units[ui].warmth <= 0 {
                    w.set_warmth(ui, 4);
                    let u = &mut w.units[ui];
                    u.state = State::Stray;
                    u.frozen_since = None;
                    u.moving = false;
                    u.last_tiles.clear();
                }
            }
        }
    }

    // unselect penguins that can't be ordered
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    for u in sel {
        let un = &w.units[u];
        if un.warmth < 5 || matches!(un.state, State::Frozen | State::Entering | State::InBuilding) {
            w.selected.remove(&u);
            w.units[u].selected = false;
        }
    }

    gfx::clear((0, 0, 0));
    let v = w.view;
    gfx::begin_world(v.x, v.y, v.zoom);
    gfx::color4f(1.0, 1.0, 1.0, 1.0);
    s.terrain.draw(v.x, v.y, vw, vh);

    // dynamic map objects
    for d in &w.dmos {
        if d.hidden {
            continue;
        }
        let x = (d.position.0 * 32) as f64;
        let y = (d.position.1 * 32) as f64 - w.elevation(d.position.0, d.position.1) * ELEV_MULT;
        let t = gfx::texture(&d.image_name);
        let (tw, th) = (t.w as f64, t.h as f64);
        if x + tw > v.x && x < v.x + vw && y + th > v.y && y < v.y + vh + th {
            w.draw_buffer.push(Rendered::tex(t, x, y, 1, [1.0; 4]));
        }
    }
    // resources (trees, fish, ...)
    for r in &w.resources {
        let t = r.texture();
        if r.x + t.w as f64 > v.x && r.x < v.x + vw && r.y + t.h as f64 > v.y && r.y < v.y + vh {
            w.draw_buffer.push(Rendered::tex(t, r.x, r.y, 1, [1.0; 4]));
        }
    }

    let view4 = (v.x, v.y, vw, vh);
    crate::wall::draw(w, view4);
    let detail = settings::option_int("snowballdetail");
    if detail >= 2 {
        w.imprints.snowball.draw(now, view4);
        w.imprints.snowball_ice.draw(now, view4);
    }
    if w.footprints {
        w.imprints.foot.draw(now, view4);
    }

    // move and draw the penguins
    for ui in 0..w.units.len() {
        // when it froze (the server's brain sets it too; clients need it
        // for the thaw countdown)
        if !w.is_server() && w.units[ui].switch_pending && w.units[ui].switch_done_at.is_some_and(|t| (w.now().wrapping_sub(t) as i32) >= 0) {
            w.units[ui].switch_pending = false;
            w.units[ui].switch_done_at = None;
        }
        if w.units[ui].state == State::Frozen {
            if w.units[ui].frozen_since.is_none() {
                w.units[ui].frozen_since = Some(w.now());
            }
        } else if !w.is_server() {
            w.units[ui].frozen_since = None;
        }
        unit::move_unit(w, ui);
        unit::animate(w, ui, dt as f64);
        let (cx, cy) = w.units[ui].sprite_xy(gfx::time());
        if cx + 64.0 > v.x && cx < v.x + vw + 64.0 && cy + 64.0 > v.y && cy < v.y + vh + 64.0 {
            draw_unit(w, ui);
        }
    }
    // igloos
    let mut visible = Vec::new();
    for bi in 0..w.buildings.len() {
        let (bx, by) = ((w.buildings[bi].x * 32) as f64, (w.buildings[bi].y * 32) as f64);
        if bx + 256.0 > v.x && bx < v.x + vw && by + 256.0 > v.y && by < v.y + vh + 246.0 {
            draw_building(w, bi);
            visible.push(bi);
        }
    }
    let mut buf = std::mem::take(&mut w.draw_buffer);
    buf.sort_by(|a, b| a.key.partial_cmp(&b.key).unwrap_or(std::cmp::Ordering::Equal));
    for r in &buf {
        let DrawItem::Tex { tex, x, y, color } = &r.item;
        tex.draw(*x, *y, *color, 1.0);
    }
    buf.clear();
    w.draw_buffer = buf;

    if settings::get_option("smoke") == "1" {
        for bi in visible {
            igloo::draw_smoke(w, bi);
        }
    }
    if detail == 3 {
        let tex = w.tex.snowparticle;
        let t = gfx::time();
        w.particles.retain(|p| p.run(t, &tex));
    }

    // snowballs
    let mut i = 0;
    while i < w.snowballs.len() {
        if snowball::move_snowball(w, i) {
            w.snowballs.remove(i);
        } else {
            let sb = &w.snowballs[i];
            w.tex.snowball[0].draw(sb.x, sb.y, [1.0; 4], 1.0);
            w.tex.snowball[1].draw(sb.shadow_x, sb.shadow_y, [1.0; 4], 1.0);
            i += 1;
        }
    }

    crate::wall::draw_bars(w, view4);
    w.imprints.moves.draw(now, view4);
    w.imprints.run_moves.draw(now, view4);
    w.imprints.gather_moves.draw(now, view4);

    // selection box
    if let (Some((x, y)), Some((x2, y2))) = (w.select_box_start, w.select_box_end) {
        gfx::fill_rect(x, y, x2 - x, y2 - y, Some([0.0, 0.3, 0.3, 0.3]));
    }

    let mut darkness = ((s.time - 44).abs() * 2 - 20).max(0) as f32 / 255.0;
    darkness = darkness.min(1.0);
    w.flakes.run(darkness > 0.05, gfx::time(), &mut w.rng, view4);

    // the HUD, at screen resolution
    gfx::begin_ui();
    let (sw, sh) = (w.view.w as f64, w.view.h as f64);
    if settings::get_option("daynight") == "1" {
        gfx::fill_rect(0.0, 0.0, sw, sh, Some([0.0, 0.0, 0.0, darkness]));
    }
    if w.chat.chatting {
        gfx::fill_rect(0.0, 0.0, sw, 60.0, Some([0.0, 0.0, 0.0, 0.7]));
        if !w.chat.help.is_empty() {
            w.chat_font.render(10.0, 30.0, &w.chat.help, [0.65, 0.65, 0.65, 1.0]);
        }
        w.chat_font.render(10.0, 0.0, &format!("{}_", w.chat.msg), [1.0; 4]);
    }
    draw_messages(w);
    hud::draw_panel(w);
    s.minimap.draw(w);
    if w.show_fps {
        w.chat_font.render(sw - 130.0, sh - 30.0, &format!("FPS: {}", s.fps.round() as i64), [1.0, 1.0, 0.3, 1.0]);
    }
    draw_thaw_countdown(s, w, now);
    draw_drop_countdown(w, now);
    crate::hud::draw_squads(w);
    crate::hud::draw_wall_mode(w);
    crate::hud::draw_alert(w);
    crate::hud::draw_crystal_race(w);
    crate::hud::draw_alliances(w);
    draw_countdown(s, w);
    if w.spectating {
        // a banner at the bottom
        let text = "Spectating - you were defeated  (Esc: menu)";
        let (tw, th) = w.chat_font.size_of(text);
        let (x, y) = ((sw - tw as f64) / 2.0, 12.0);
        gfx::fill_rect(x - 12.0, y - 6.0, tw as f64 + 24.0, th as f64 + 12.0, Some([0.0, 0.0, 0.0, 0.55]));
        w.chat_font.render(x, y, text, [1.0, 1.0, 1.0, 1.0]);
    }
    if let Some(m) = &s.game_menu {
        m.draw();
    }
    if s.use_cursor {
        let (mx, my) = gfx::mouse_pos();
        s.cursor.draw(mx as f64, sh - 64.0 - my as f64, [1.0; 4], 1.0);
    }
    // fade the loading screen out
    if s.loading_alpha > 0.0 {
        s.loading_alpha -= 0.05;
        gfx::fill_rect(0.0, 0.0, sw, sh, Some([0.0, 0.0, 0.0, s.loading_alpha]));
        backgrounds::draw("loading", s.loading_alpha.max(0.0));
    }
    gfx::present();
}

/// Chat and notices (bottom left; old ones fade out).
pub fn draw_messages(w: &mut World) {
    let now = w.now();
    let lh = w.chat_font.size as f64 / 0.63;
    let mut y = lh;
    let mut remove_first = false;
    let mut first = false;
    let vh = w.view.h as f64;
    let font = w.chat_font;
    for m in w.messages.iter_mut() {
        let h = lh * m.num_lines as f64;
        let mut timep = now.wrapping_sub(m.time) as f64;
        m.animate();
        let mut alpha = m.alpha;
        if first && !m.force_close {
            m.time = now;
        } else if timep > 4000.0 || m.force_close {
            if m.force_close && timep < 4000.0 {
                m.time = now.wrapping_sub(4000);
                timep = 4000.0;
            }
            alpha = 1.0 - (timep - 4000.0) / 1000.0;
            y -= h - alpha * h;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        first = true;
        font.render(10.0, vh - y, &m.msg, [m.color[0], m.color[1], m.color[2], alpha as f32]);
        if alpha == 0.0 {
            remove_first = true;
        }
        y += h;
    }
    if remove_first && !w.messages.is_empty() {
        w.messages.remove(0);
    }
}

/// How long a hit penguin flashes blue (ms).
const HIT_FLASH_MS: u32 = 160;

/// One tick of the time of day (88 ticks a day: 4 minutes).
pub const TIME_TICK_MS: f64 = 2.0 * 1000.0 * 60.0 / 44.0;

/// Milliseconds until noon, when every frozen penguin thaws.
pub fn ms_until_noon(s: &Session, now: u32) -> u32 {
    // ticks still to come until the time of day is 44
    let k = (43 - s.time).rem_euclid(88) + 1;
    let next = s.last_time_tick as f64 + TIME_TICK_MS - now as f64;
    (next.max(0.0) + (k - 1) as f64 * TIME_TICK_MS) as u32
}

/// When this frozen penguin thaws, in milliseconds from now: next to its
/// own igloo after `THAW_AT_IGLOO_MS`, anywhere at noon.
pub fn thaw_in(s: &Session, w: &World, ui: usize, now: u32) -> u32 {
    let noon = ms_until_noon(s, now);
    let u = &w.units[ui];
    if unit::near_own_igloo(w, ui, 3) {
        let since = u.frozen_since.unwrap_or(now);
        let igloo = unit::THAW_AT_IGLOO_MS.saturating_sub(now.wrapping_sub(since));
        // (the brain looks every few hundred milliseconds)
        return igloo.min(noon);
    }
    noon
}

/// The frozen penguin under the mouse pointer, if any.
pub fn frozen_under_mouse(w: &World) -> Option<usize> {
    let (mx, my) = gfx::mouse_pos();
    let (px, py) = (w.view.x + mx as f64 / w.view.zoom, w.view.y + my as f64 / w.view.zoom);
    let t = gfx::time();
    // the one drawn on top (lowest on the screen) wins
    (0..w.units.len())
        .filter(|&u| w.units[u].state == State::Frozen)
        .filter(|&u| {
            let (x, y) = w.units[u].sprite_xy(t);
            px >= x && px < x + 32.0 && py >= y && py < y + 32.0
        })
        .max_by(|&a, &b| w.units[a].sprite_xy(t).1.partial_cmp(&w.units[b].sprite_xy(t).1).unwrap_or(std::cmp::Ordering::Equal))
}

/// "Thaws in 0:23" over the frozen penguin under the pointer and over the
/// one clicked last.
fn draw_thaw_countdown(s: &Session, w: &mut World, now: u32) {
    if w.inspected.is_some_and(|u| u >= w.units.len() || w.units[u].state != State::Frozen) {
        w.inspected = None;
    }
    let hover = if s.game_menu.is_none() { frozen_under_mouse(w) } else { None };
    let mut show: Vec<usize> = hover.into_iter().collect();
    if let Some(u) = w.inspected {
        if Some(u) != hover {
            show.push(u);
        }
    }
    let sh = w.view.h as f64;
    let t = gfx::time();
    for ui in show {
        let left = thaw_in(s, w, ui, now);
        let secs = (left + 999) / 1000;
        let text = if secs == 0 { "Thawing...".to_string() } else { if secs >= 60 {
            format!("Thaws in {}m {:02}s", secs / 60, secs % 60)
        } else {
            format!("Thaws in {}s", secs)
        } };
        let (x, y) = w.units[ui].sprite_xy(t);
        let cx = (x + 16.0 - w.view.x) * w.view.zoom;
        let top = (y - w.view.y) * w.view.zoom;
        let (tw, th) = w.chat_font.size_of(&text);
        let (tw, th) = (tw as f64, th as f64);
        let bx = (cx - tw / 2.0).round();
        let by = (sh - top + 8.0).round();
        gfx::fill_rect(bx - 8.0, by - 4.0, tw + 16.0, th + 8.0, Some([0.07, 0.09, 0.16, 0.8]));
        w.chat_font.render(bx, by, &text, [0.75, 0.92, 1.0, 1.0]);
    }
}

/// Top middle: "Power-ups in 42s" (and "New power-ups!" right after a drop).
fn draw_drop_countdown(w: &World, now: u32) {
    let Some(at) = w.powerup_drop_at else { return };
    let (sw, sh) = (w.view.w as f64, w.view.h as f64);
    let fresh = w.powerup_dropped_at.is_some_and(|t| now.wrapping_sub(t) < 4000);
    let left = at.wrapping_sub(now) as i32;
    let secs = (left.max(0) as u32).div_ceil(1000);
    let text = if fresh {
        "New power-ups on the map!".to_string()
    } else if left <= 0 {
        "Power-ups dropping...".to_string()
    } else {
        format!("Power-ups in {}s", secs)
    };
    let soon = !fresh && secs <= 5;
    let font = w.chat_font;
    let (tw, th) = font.size_of(&text);
    let (tw, th) = (tw as f64, th as f64);
    // (the orb fills about 18 of the picture's 32 pixels, centred at 15,16)
    let scale = 1.25;
    let icon = 18.0 * scale;
    let width = icon + 10.0 + tw;
    let x = ((sw - width) / 2.0).round();
    let y = sh - th - 14.0;
    gfx::fill_rect(x - 12.0, y - 6.0, width + 24.0, th + 12.0, Some([0.07, 0.09, 0.16, 0.7]));
    let orb = gfx::texture("plugins/powerup/powerup.png");
    orb.draw((x - 6.0 * scale).round(), (y + th / 2.0 - 16.0 * scale).round(), [1.0; 4], scale);
    let color = if fresh {
        [1.0, 0.95, 0.55, 1.0]
    } else if soon {
        [1.0, 0.85, 0.4, 1.0]
    } else {
        [0.85, 0.93, 1.0, 1.0]
    };
    font.render(x + icon + 10.0, y, &text, color);
}

/// The start countdown (3, 2, 1, Go!) over the middle of the screen.
fn draw_countdown(s: &Session, w: &World) {
    let now = crate::sdl::ticks();
    let (sw, sh) = (w.view.w as f64, w.view.h as f64);
    let (text, frac, color) = if let Some(t) = s.countdown_until {
        let left = t.wrapping_sub(now) as i32;
        if left <= 0 {
            return;
        }
        let n = (left as u32).div_ceil(1000);
        // (each number fades as its second runs out)
        let frac = (left as u32 % 1000) as f32 / 1000.0;
        (n.to_string(), if frac == 0.0 { 1.0 } else { frac }, [1.0, 1.0, 1.0])
    } else if let Some(t) = s.go_until {
        let left = t.wrapping_sub(now) as i32;
        if left <= 0 {
            return;
        }
        ("Go!".to_string(), left as f32 / crate::game::GO_MS as f32, [0.55, 1.0, 0.55])
    } else {
        return;
    };
    if s.countdown_until.is_some() {
        gfx::fill_rect(0.0, 0.0, sw, sh, Some([0.03, 0.05, 0.12, 0.35]));
    }
    let big = w.chat_font.with_size(((sh / 4.5).round() as u32).clamp(60, 220));
    let (tw, th) = big.size_of(&text);
    let a = (0.35 + 0.65 * frac).min(1.0);
    let (x, y) = ((sw - tw as f64) / 2.0, (sh - th as f64) / 2.0);
    // a dark outline, then the text
    for (dx, dy) in [(-3.0, 0.0), (3.0, 0.0), (0.0, -3.0), (0.0, 3.0), (-2.0, -2.0), (2.0, 2.0), (-2.0, 2.0), (2.0, -2.0)] {
        big.render(x + dx, y + dy, &text, [0.05, 0.08, 0.2, a * 0.8]);
    }
    big.render(x, y, &text, [color[0], color[1], color[2], a]);
    if s.countdown_until.is_some() {
        let sub = "Get ready!";
        let (stw, _) = w.chat_font.size_of(sub);
        w.chat_font.render((sw - stw as f64) / 2.0, y - 34.0, sub, [1.0, 1.0, 1.0, 0.9]);
    }
}

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
        w.draw_buffer.push(Rendered { key, item: DrawItem::Tex { tex: t, x, y, color: [1.0; 4] } });
    }
    if let Some(o) = outfit {
        w.draw_buffer.push(Rendered { key, item: DrawItem::Tex { tex: o, x, y, color: [color[0], color[1], color[2], 1.0] } });
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
        w.stat_font.render(x, y, &g.to_string(), [1.0; 4]);
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
    let now = clock::now();
    gfx::set_time(now);
    let (vw, vh) = (w.view.scale_w() as f64, w.view.scale_h() as f64);

    // the time of day: half a day takes 2 minutes; resources come back at
    // midnight, frozen penguins thaw at noon
    if now as f64 > s.last_time_tick as f64 + 2.0 * 1000.0 * 60.0 / 44.0 {
        s.last_time_tick = now;
        s.time += 1;
        if s.time == 88 {
            s.time = 0;
            for r in 0..w.resources.len() {
                let q = w.rng.randint(5, 10);
                w.set_quantity(r, q);
            }
        }
        if s.time == 44 {
            for ui in 0..w.units.len() {
                if w.units[ui].warmth <= 0 {
                    w.set_warmth(ui, 4);
                    w.units[ui].state = State::Stray;
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
    if w.draw_minimap {
        s.minimap.draw(w);
    }
    if w.show_fps {
        w.chat_font.render(sw - 130.0, sh - 30.0, &format!("FPS: {}", s.fps.round() as i64), [1.0, 1.0, 0.3, 1.0]);
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

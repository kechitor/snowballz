//! Igloos (buildings/building.py + buildings/igloo.py): warming penguins
//! up, being taken over, smoke.

use crate::world::*;

pub fn is_in_range(w: &World, bi: usize, x: i32, y: i32) -> bool {
    let b = &w.buildings[bi];
    if b.construction < 100 {
        return false;
    }
    !(x <= b.x - IGLOO_RANGE || x >= b.x + IGLOO_RANGE || y <= b.y - IGLOO_RANGE || y >= b.y + IGLOO_RANGE)
}

pub fn can_enter(w: &World, bi: usize, ui: usize) -> bool {
    let u = &w.units[ui];
    let owner = w.buildings[bi].player;
    if u.player != owner {
        u.warmth >= 5
    } else {
        true
    }
}

fn unit_exit(w: &mut World, bi: usize, ui: usize) -> Step {
    w.units[ui].state = State::Stray;
    w.buildings[bi].units_in_building.remove(&ui);
    w.units[ui].set_go_to_building(None);
    Step::Go(-1, -1)
}

/// A penguin inside the igloo: warm up, then leave (or take the igloo
/// over, for an enemy).
pub fn in_building(w: &mut World, bi: usize, ui: usize) -> Step {
    let now = w.now();
    if w.buildings[bi].jeopardy {
        let up = w.units[ui].player;
        if up != w.buildings[bi].player {
            if w.buildings[bi].jeopardy_count.wrapping_add(TAKEOVER_TIME) <= now {
                // takeover!
                let r = w.buildings[bi].region;
                w.set_region_player(r, up);
                w.set_jeopardy(bi, false);
            }
        }
        return Step::Wait;
    }
    let (warmth, max) = (w.units[ui].warmth, w.units[ui].max_warmth);
    if warmth == max {
        // warm: out you go
        w.buildings[bi].units_in_building.remove(&ui);
        w.units[ui].state = State::Stray;
        w.units[ui].set_go_to_building(None);
        return Step::Go(-1, -1);
    }
    let counter = w.units[ui].counter as f64;
    let wm = warmth.max(1) as f64;
    if counter + 120000.0 / wm <= now as f64 {
        if warmth > max - 2 {
            w.units[ui].max_warmth = 10;
            w.units[ui].snowballs = 2;
            return unit_exit(w, bi, ui);
        } else if w.num_storage(bi, ResKind::Fish) > 0 {
            w.set_warmth(ui, max);
            w.add_storage(bi, ResKind::Fish, -1);
            w.units[ui].snowballs = 2;
            return unit_exit(w, bi, ui);
        } else if counter + 200000.0 / wm <= now as f64 {
            w.set_warmth(ui, max);
            w.units[ui].snowballs = 2;
            return unit_exit(w, bi, ui);
        }
    }
    Step::Wait
}

/// A penguin goes in: an enemy puts the igloo in jeopardy, a warm
/// snowballer of the owner saves it and kicks enemies out.
pub fn unit_enter(w: &mut World, bi: usize, ui: usize) {
    let up = w.units[ui].player;
    let owner = w.buildings[bi].player;
    if up != owner {
        w.buildings[bi].player_taking_over = up;
        w.set_jeopardy(bi, true);
        w.buildings[bi].jeopardy_count = w.now();
    }
    if w.units[ui].warmth > 4 {
        let snowballer = w.units[ui].kind == UnitKind::Snowballer;
        if w.buildings[bi].jeopardy && up == owner && snowballer {
            // building saved!
            w.set_jeopardy(bi, false);
        }
        if snowballer {
            let kick: Vec<usize> = w.buildings[bi]
                .units_in_building
                .iter()
                .copied()
                .filter(|&o| w.units[o].player != up && w.units[o].player != owner)
                .collect();
            for o in kick {
                w.set_warmth(o, 4);
                unit_exit(w, bi, o);
            }
        }
    }
    if up == owner {
        w.units[ui].igloo = bi;
    }
}

/// Draw the smoke of the chimney (and make new puffs).
pub fn draw_smoke(w: &mut World, bi: usize) {
    let smoke_tex = w.tex.smoke;
    let elev = w.tiles[w.buildings[bi].tile].elevation;
    let (bx, by) = (w.buildings[bi].x, w.buildings[bi].y);
    let old = std::mem::take(&mut w.buildings[bi].smoke);
    let mut new = Vec::with_capacity(old.len() + 1);
    for ((x, y), life) in old {
        let dx = (bx * 32) as f64 + x;
        let dy = (by * 32) as f64 + y - elev * ELEV_MULT;
        let (opc, s) = if life > 0 { (life as f64 / 60.0, life as f64 / 60.0) } else { (0.0, 0.0) };
        let s = 1.0 - s + 0.7;
        smoke_tex.draw(dx, dy, [1.0, 1.0, 1.0, opc as f32], s);
        let a = (w.rng.randint(0, 1) as f64 - 0.5).max(0.0);
        let life = life - 1;
        if life > 0 {
            new.push(((x + a, y - 0.5), life));
        }
    }
    if !w.buildings[bi].units_in_building.is_empty() {
        let now = w.now();
        if now > w.buildings[bi].smoke_counter.wrapping_add(100) {
            w.buildings[bi].smoke_counter = now;
            let sx = SMOKE_START.0 + w.rng.randint(-2, 2) as f64;
            new.push(((sx, SMOKE_START.1), 60));
        }
    }
    w.buildings[bi].smoke = new;
}

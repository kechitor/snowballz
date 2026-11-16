//! What penguins do (unit.py, and the network halves MasterUnit /
//! SlaveUnit).  Everything works on the world through unit indices.

use crate::effects::Lerp;
use crate::igloo;
use crate::net::{self, Msg};
use crate::plugins::{self, Event};
use crate::snowball::Snowball;
use crate::world::*;

pub const THAW_AT_IGLOO_MS: u32 = 30000;
/// The walk animation advances 35 steps a second, at any frame rate.
pub const ANIM_STEP_MS: f64 = 1000.0 / 35.0;

fn turn_right(d: (i32, i32)) -> (i32, i32) {
    match d {
        (0, 1) => (-1, 1),
        (-1, 1) => (-1, 0),
        (-1, 0) => (-1, -1),
        (-1, -1) => (0, -1),
        (0, -1) => (1, -1),
        (1, -1) => (1, 0),
        (1, 0) => (1, 1),
        _ => (0, 1),
    }
}

fn turn_left(d: (i32, i32)) -> (i32, i32) {
    match d {
        (-1, -1) => (-1, 0),
        (-1, 0) => (-1, 1),
        (-1, 1) => (0, 1),
        (0, -1) => (-1, -1),
        (0, 1) => (1, 1),
        (1, -1) => (0, -1),
        (1, 0) => (1, -1),
        _ => (1, 0),
    }
}

/// Milliseconds to walk to the next tile (slower uphill).
pub fn walk_speed(w: &World, ui: usize) -> f64 {
    let u = &w.units[ui];
    let here = w.elevation(u.x, u.y);
    let d = match w.tile(u.x + u.direction.0, u.y + u.direction.1) {
        Some(t) => t.elevation - here,
        None => 0.0,
    };
    800.0 + d * 50.0
}

pub fn near_own_igloo(w: &World, ui: usize, distance: i32) -> bool {
    let u = &w.units[ui];
    let b = &w.buildings[u.igloo];
    let dx = (b.x - u.x).max(0).max(u.x - (b.x + IGLOO_SIZE - 1));
    let dy = (b.y - u.y).max(0).max(u.y - (b.y + IGLOO_SIZE - 1));
    dx.max(dy) <= distance
}

/// Can the penguin step in this direction?
fn try_direction(w: &mut World, ui: usize, d: (i32, i32)) -> bool {
    let (x, y) = (w.units[ui].x, w.units[ui].y);
    let pos = (x + d.0, y + d.1);
    let Some(ti) = w.tile_index(pos.0, pos.1) else { return false };
    if w.units[ui].state != State::Entering {
        if let Some(b) = w.tiles[ti].building {
            // penguins can walk on the top row of a building
            if w.buildings[b].y != pos.1 {
                return false;
            }
        }
        if w.dmos.iter().any(|o| o.obstruction && o.position == (x, y)) {
            return false;
        }
        for u in w.units_around(x, y) {
            // frozen penguins don't block the way (a group of them frozen
            // around an igloo door used to lock everybody out)
            if u != ui && w.units[u].state != State::Frozen && (w.units[u].x, w.units[u].y) == pos {
                return false;
            }
        }
    } else {
        w.units[ui].state = State::InBuilding;
    }
    if w.tiles[ti].resource.is_some() {
        return false;
    }
    let u = &mut w.units[ui];
    if u.last_tiles.len() >= 30 {
        u.last_tiles.remove(0);
    }
    !u.last_tiles.contains(&pos)
}

/// Move (and send the new position to the clients).
pub fn move_unit(w: &mut World, ui: usize) {
    if w.role == Role::Client {
        slave_move(w, ui);
        return;
    }
    let pos = (w.units[ui].x, w.units[ui].y);
    local_move(w, ui);
    if w.role == Role::Server {
        let u = &w.units[ui];
        if pos != (u.x, u.y)
            || (u.state != u.last_state_sent && !matches!(u.state, State::Throwing | State::GettingSnowball))
        {
            let m = Msg::UnitPosition {
                unit_id: ui as u16,
                x: u.x as i16,
                y: u.y as i16,
                dir_x: u.direction.0 as i8,
                dir_y: u.direction.1 as i8,
                state_id: u.state.index(),
                holding_id: ResKind::holding_id(u.holding),
            };
            w.send(m);
        }
    }
    let s = w.units[ui].state;
    w.units[ui].last_state_sent = s;
}

fn local_move(w: &mut World, ui: usize) {
    let start = (w.units[ui].x, w.units[ui].y);
    let now = w.now();
    if !w.units[ui].moving {
        let wait = w.rng.randint(200, 400) as u32;
        if now > w.units[ui].fail_time.wrapping_add(wait) {
            if check_direction(w, ui) == Step::Fail {
                w.units[ui].fail_time = w.now();
            }
        }
    } else {
        let passed = now.wrapping_sub(w.units[ui].counter) as f64;
        if passed / walk_speed(w, ui) >= 1.0 {
            check_direction(w, ui);
        }
    }
    let end = (w.units[ui].x, w.units[ui].y);
    if start != end {
        move_sprites(w, ui, start, end);
    }
}

pub fn move_sprites(w: &mut World, ui: usize, start: (i32, i32), end: (i32, i32)) {
    let sx = (start.0 * 32) as f64;
    let sy = (start.1 * 32) as f64 - w.elevation(start.0, start.1) * ELEV_MULT;
    let ex = (end.0 * 32) as f64;
    let ey = (end.1 * 32) as f64 - w.elevation(end.0, end.1) * ELEV_MULT;
    let ws = walk_speed(w, ui);
    let t = crate::gfx::time();
    let u = &mut w.units[ui];
    u.sx = Lerp::new(sx, ex, t, ws);
    u.sy = Lerp::new(sy, ey, t, ws);
}

/// Step if the brain says so; returns Go if the penguin moved.
fn check_direction(w: &mut World, ui: usize) -> Step {
    let d = find_path(w, ui);
    match d {
        Step::Go(dx, dy) => {
            let (x, y) = (w.units[ui].x, w.units[ui].y);
            w.units[ui].last_tiles.push((x, y));
            let (nx, ny) = (x + dx, y + dy);
            w.units[ui].x = nx;
            w.units[ui].y = ny;
            plugins::fire(w, Event::PenguinMove(ui, nx, ny));
            update_node(w, ui);
            let now = w.now();
            let u = &mut w.units[ui];
            u.direction = (dx, dy);
            u.moving = true;
            u.counter = now;
            d
        }
        other => {
            w.units[ui].moving = false;
            other
        }
    }
}

/// Keep the unit in the right 7x7 node (for "who is around" lookups).
pub fn update_node(w: &mut World, ui: usize) {
    let (x, y) = (w.units[ui].x, w.units[ui].y);
    let node = (x.div_euclid(UNIT_NODE), y.div_euclid(UNIT_NODE));
    if w.units[ui].prv_node != Some(node) {
        if let Some(old) = w.units[ui].prv_node {
            if let Some(n) = w.unit_nodes.get_mut(&old) {
                n.remove(&ui);
            }
        }
        w.unit_nodes.entry(node).or_default().insert(ui);
        w.units[ui].prv_node = Some(node);
    }
}

fn find_path(w: &mut World, ui: usize) -> Step {
    let d = stray(w, ui);
    let Step::Go(dx, dy) = d else { return d };
    if (dx, dy) == (0, 0) {
        return Step::Fail;
    }
    let mut left = turn_left((dx, dy));
    let mut right = (dx, dy);
    loop {
        if try_direction(w, ui, right) {
            return Step::Go(right.0, right.1);
        }
        if try_direction(w, ui, left) {
            return Step::Go(left.0, left.1);
        }
        right = turn_right(right);
        if right == left {
            w.units[ui].last_tiles.clear();
            return Step::Fail;
        }
        left = turn_left(left);
    }
}

/// The penguin's brain ("This code is a MESS!!! Read at your own risk!").
fn stray(w: &mut World, ui: usize) -> Step {
    let time = w.now();
    let (x, y) = (w.units[ui].x, w.units[ui].y);
    let here = w.tile_index(x, y).unwrap_or(0);

    if w.units[ui].warmth == 0 && w.tiles[here].building_root {
        w.set_warmth(ui, 1);
    }
    if w.units[ui].warmth < 1 {
        let since = *w.units[ui].frozen_since.get_or_insert(time);
        w.units[ui].state = State::Frozen;
        // frozen next to its own igloo: the igloo thaws it after a while
        // (out in the field penguins wait for the noon thaw)
        if time.wrapping_sub(since) >= THAW_AT_IGLOO_MS && near_own_igloo(w, ui, 3) {
            w.set_warmth(ui, 4);
            w.units[ui].state = State::Stray;
            w.units[ui].frozen_since = None;
        }
        return Step::Fail;
    }
    w.units[ui].frozen_since = None;

    // states that can't be interrupted
    match w.units[ui].state {
        State::Unloading => return Step::Wait,
        State::Entering => {
            w.units[ui].last_tiles.clear();
            w.units[ui].counter = w.now();
            if let Some(b) = w.units[ui].go_to_building() {
                w.buildings[b].units_in_building.insert(ui);
                igloo::unit_enter(w, b, ui);
            }
            return Step::Go(1, 1);
        }
        State::InBuilding => {
            return match w.units[ui].go_to_building() {
                Some(b) => igloo::in_building(w, b, ui),
                None => {
                    w.units[ui].state = State::Stray;
                    Step::Wait
                }
            };
        }
        State::Throwing => {
            if w.units[ui].current_frame < 9 {
                return Step::Wait;
            }
            w.units[ui].state = State::Stray;
            w.units[ui].targeted_unit = None;
        }
        _ => {}
    }

    let kind = w.units[ui].kind;
    let cfg = w.unit_cfg(kind);
    if w.units[ui].warmth < 5 {
        // too cold: clear the job and go home
        let u = &mut w.units[ui];
        u.state = State::Stray;
        u.gathering_from = None;
        u.target_resource = None;
        u.target_pos = None;
        let ig = u.igloo;
        if igloo::can_enter(w, ig, ui) {
            w.units[ui].set_go_to_building(Some(ig));
        }
    } else {
        if w.units[ui].state == State::GettingSnowball {
            if w.units[ui].counter.wrapping_add(cfg.create_snowball_time) <= w.now() {
                w.units[ui].snowballs += 1;
                plugins::fire(w, Event::PenguinReload(ui));
                w.units[ui].state = State::Stray;
            } else {
                return Step::Wait;
            }
        }
        if w.units[ui].state == State::Gathering && w.units[ui].target_pos.is_none() {
            w.units[ui].last_tiles.clear();
            let res = w.units[ui].gathering_from.and_then(|t| w.tiles[t].resource);
            let q = res.map(|r| w.resources[r].quantity).unwrap_or(0);
            if q == 0 {
                // the resource ran out while gathering
                let u = &mut w.units[ui];
                u.gathering_from = None;
                u.target_resource = None;
                u.state = State::Stray;
                return stray_at_igloo(w, ui, false);
            }
            if w.units[ui].counter.wrapping_add(cfg.gather_time) <= w.now() {
                // finished gathering
                let r = res.unwrap();
                w.set_quantity(r, q - 1);
                let kind = w.resources[r].kind;
                let u = &mut w.units[ui];
                u.holding = Some(kind);
                u.gathering_from = None;
                u.target_resource = None;
                u.state = State::Stray;
                plugins::fire(w, Event::PenguinGathered(ui));
                let ig = w.units[ui].igloo;
                return move_to_building(w, ui, ig);
            }
            return Step::Wait;
        } else if w.units[ui].state == State::Gathering {
            let u = &mut w.units[ui];
            u.gathering_from = None;
            u.target_resource = None;
            u.state = State::Stray;
        }
    }

    let ig = w.units[ui].igloo;
    if w.buildings[ig].player.is_none() {
        return stray_at_igloo(w, ui, false);
    }
    if kind == UnitKind::Snowballer && w.buildings[ig].jeopardy {
        w.units[ui].set_go_to_building(Some(ig));
    }
    if let Some(b) = w.units[ui].go_to_building() {
        if (x, y) == (w.buildings[b].x, w.buildings[b].y) {
            if igloo::can_enter(w, b, ui) {
                w.units[ui].state = State::Entering;
                return Step::Wait;
            }
            w.units[ui].set_go_to_building(None);
        }
    }
    if let Some(b) = w.units[ui].go_to_building() {
        return move_to_building(w, ui, b);
    }
    let t = &w.tiles[here];
    let own_root = t.building_root && t.building.map(|b| w.buildings[b].player) == Some(w.buildings[ig].player);
    if (kind == UnitKind::Snowballer && own_root) || t.building.is_some() {
        return stray_at_igloo(w, ui, true);
    }
    match kind {
        UnitKind::Snowballer => snowballer_work(w, ui),
        UnitKind::Worker => worker_work(w, ui),
    }
}

pub fn direction_to_tile(w: &World, ui: usize, tx: i32, ty: i32) -> (i32, i32) {
    let u = &w.units[ui];
    ((tx - u.x).signum(), (ty - u.y).signum())
}

fn throw_snowball(w: &mut World, ui: usize, target: usize) {
    let s = Snowball::throw_at_target(w, ui, target);
    let m = Msg::Snowball {
        unit_id: ui as u16,
        start_x: net::clamp_i32(s.start_x),
        start_y: net::clamp_i32(s.start_y),
        end_x: net::clamp_i32(s.end_x),
        end_y: net::clamp_i32(s.end_y),
        end_tile_x: w.tiles[s.end_tile].x as u16,
        end_tile_y: w.tiles[s.end_tile].y as u16,
        snowballs_left: 0,
        px: w.units[ui].x as u16,
        py: w.units[ui].y as u16,
    };
    w.snowballs.push(s);
    w.units[ui].snowballs -= 1;
    w.units[ui].state = State::Throwing;
    let (tx, ty) = (w.units[target].x, w.units[target].y);
    w.units[ui].direction = direction_to_tile(w, ui, tx, ty);
    w.units[ui].current_frame = 1;
    if w.role == Role::Server {
        let left = net::clamp_i16(w.units[ui].snowballs as f64);
        if let Msg::Snowball { unit_id, start_x, start_y, end_x, end_y, end_tile_x, end_tile_y, px, py, .. } = m {
            w.send(Msg::Snowball { unit_id, start_x, start_y, end_x, end_y, end_tile_x, end_tile_y, snowballs_left: left, px, py });
        }
        w.units[ui].last_state_sent = State::Throwing;
    }
}

fn random_step(w: &mut World) -> (i32, i32) {
    let dx = w.rng.randint(-1, 1);
    let dy = w.rng.randint(-1, 1);
    (dx, dy)
}

fn stray_at_igloo(w: &mut World, ui: usize, force: bool) -> Step {
    let ig = w.units[ui].igloo;
    if igloo::is_in_range(w, ig, w.units[ui].x, w.units[ui].y) {
        let will = if force { 1 } else { w.rng.randint(1, 5) };
        if will == 1 {
            let (dx, dy) = random_step(w);
            if dx == 0 && dy == 0 {
                return Step::Fail;
            }
            return Step::Go(dx, dy);
        }
        Step::Fail
    } else {
        move_to_building(w, ui, ig)
    }
}

/// Give a penguin a job: walk to `pos`, gather there or enter a building.
pub fn set_job(w: &mut World, ui: usize, pos: (i32, i32), run_mode: bool, group_size: usize) {
    let Some(ti) = w.tile_index(pos.0, pos.1) else { return };
    if matches!(w.units[ui].state, State::Entering | State::InBuilding) {
        return;
    }
    let u = &mut w.units[ui];
    u.run_mode = run_mode;
    u.last_tiles.clear();
    u.set_go_to_building(None);
    u.target_pos = None;
    let t = &w.tiles[ti];
    let res = t.resource.filter(|&r| w.resources[r].kind.gatherable());
    if w.units[ui].kind == UnitKind::Worker && res.is_some() {
        let r = res.unwrap();
        w.units[ui].going_for = Some(w.resources[r].kind);
        w.units[ui].target_resource = Some(r);
    } else if let Some(b) = t.building {
        if !w.buildings[b].units_in_building.contains(&ui) {
            w.units[ui].set_go_to_building(Some(b));
        }
    } else {
        let u = &mut w.units[ui];
        u.target_pos = Some(pos);
        u.stray_range = ((group_size / 2) as i32).clamp(0, 6);
    }
}

pub fn disban(w: &mut World, ui: usize) {
    let u = &mut w.units[ui];
    u.selected = false;
    u.set_go_to_building(None);
    u.target_pos = None;
    match u.kind {
        UnitKind::Snowballer => u.going_for = None,
        UnitKind::Worker => {
            u.going_for = Some(ResKind::Fish);
            u.target_resource = None;
        }
    }
}

pub fn move_to_building(w: &mut World, ui: usize, b: usize) -> Step {
    let (x, y) = (w.buildings[b].x, w.buildings[b].y);
    find_next_tile_to_pos(w, ui, x, y)
}

/// One step towards a tile (with a bit of randomness for diagonals).
pub fn find_next_tile_to_pos(w: &mut World, ui: usize, x: i32, y: i32) -> Step {
    let dx = (x - w.units[ui].x) as f64;
    let dy = (y - w.units[ui].y) as f64;
    let d = (dx * dx + dy * dy).sqrt();
    if d == 0.0 {
        return Step::Fail;
    }
    let (dx, dy) = (dx / d * 2f64.sqrt(), dy / d * 2f64.sqrt());
    let mut sx = 0;
    let mut sy = 0;
    if dx > 0.0 && dx > w.rng.random() {
        sx = 1;
    } else if dx < 0.0 && dx < -w.rng.random() {
        sx = -1;
    }
    if dy > 0.0 && dy > w.rng.random() {
        sy = 1;
    } else if dy < 0.0 && dy < -w.rng.random() {
        sy = -1;
    }
    if sx == 0 && sy == 0 {
        return Step::Fail;
    }
    Step::Go(sx, sy)
}

/// Enemy penguins that are close enough to throw at.
fn enemies_in_range(w: &World, ui: usize) -> Vec<usize> {
    let u = &w.units[ui];
    let mine = w.buildings[u.igloo].player;
    let here = w.elevation(u.x, u.y);
    let mut out = Vec::new();
    for o in w.units_around(u.x, u.y) {
        let ou = &w.units[o];
        if w.buildings[ou.igloo].player != mine {
            let a = 7.0 + (here - w.elevation(ou.x, ou.y));
            let (ox, oy) = (ou.x as f64, ou.y as f64);
            let (x, y) = (u.x as f64, u.y as f64);
            if !(ox > x + a || ox < x - a) && !(oy > y + a || oy < y - a) && ou.warmth > 0 && ou.state != State::InBuilding {
                out.push(o);
            }
        }
    }
    out
}

/// A snowball landed on/near this penguin; returns 2 for a direct hit, 1
/// for a near one.
pub fn detect_hit(w: &mut World, ui: usize, end_tile: usize, start_tile: Option<usize>, thrower: Option<usize>) -> i32 {
    let (x, y) = (w.tiles[end_tile].x, w.tiles[end_tile].y);
    let (ux, uy) = (w.units[ui].x, w.units[ui].y);
    let mut d = 0;
    let mut hit = 0;
    if (x, y) == (ux, uy) {
        d = 3;
        w.units[ui].last_tiles.clear();
        hit = 2;
        plugins::fire(w, Event::PenguinDirectHit(ui, d));
    } else if (x == ux - 1 || x == ux) && (y == uy - 1 || y == uy) {
        d = 1;
        w.units[ui].last_tiles.clear();
        hit = 1;
    }
    if hit > 0 && w.units[ui].warmth > 0 && w.units[ui].state != State::InBuilding {
        if let Some(st) = start_tile {
            let pos = (w.tiles[st].x, w.tiles[st].y);
            if let Some(p) = w.igloo_player(ui).and_then(|p| w.players.get_mut(&p)) {
                p.set_alert(pos, 3);
            }
        }
        let v = w.units[ui].warmth - d;
        w.set_warmth(ui, v);
        w.units[ui].last_tiles.clear();
    }
    if w.units[ui].warmth < 1 {
        w.set_warmth(ui, 0);
        plugins::fire(w, Event::PenguinFrozen(ui, thrower));
    }
    hit
}

fn is_in_range(u: &Unit, pos: (i32, i32)) -> bool {
    let r = u.stray_range;
    !(pos.0 < u.x - r || pos.0 > u.x + r || pos.1 < u.y - r || pos.1 > u.y + r)
}

fn dist(a: (i32, i32), b: (i32, i32)) -> f64 {
    (((a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)) as f64).sqrt()
}

fn snowballer_work(w: &mut World, ui: usize) -> Step {
    if !w.units[ui].run_mode {
        if w.units[ui].snowballs > 0 {
            let mut in_range = enemies_in_range(w, ui);
            if !in_range.is_empty() {
                if w.units[ui].targeted_unit.is_none() {
                    let sb: Vec<usize> = in_range.iter().copied().filter(|&o| w.units[o].kind == UnitKind::Snowballer).collect();
                    if !sb.is_empty() {
                        in_range = sb;
                    }
                    let me_pos = (w.units[ui].x, w.units[ui].y);
                    let my_player = w.igloo_player(ui);
                    let on_root = w.tile(me_pos.0, me_pos.1).map(|t| t.building_root).unwrap_or(false);
                    for o in in_range {
                        if on_root {
                            continue;
                        }
                        // don't attack penguins of an igloo that is ours
                        if w.units[o].warmth < 5 {
                            let oig = w.units[o].igloo;
                            let ours = w.buildings[oig].units_in_building.iter().any(|&un| w.igloo_player(un) == my_player);
                            if ours {
                                continue;
                            }
                        }
                        let op = (w.units[o].x, w.units[o].y);
                        match w.units[ui].targeted_unit {
                            None => w.units[ui].targeted_unit = Some(o),
                            Some(t) => {
                                let tp = (w.units[t].x, w.units[t].y);
                                if dist(op, me_pos) < dist(tp, me_pos) {
                                    w.units[ui].targeted_unit = Some(o);
                                }
                            }
                        }
                    }
                } else {
                    let t = w.units[ui].targeted_unit.unwrap();
                    if !in_range.contains(&t) {
                        w.units[ui].targeted_unit = None;
                        return Step::Wait;
                    }
                    throw_snowball(w, ui, t);
                    w.units[ui].targeted_unit = None;
                    return Step::Wait;
                }
            }
        } else if w.tile(w.units[ui].x, w.units[ui].y).map(|t| t.kind) == Some(TileKind::Snow) {
            // make a snowball
            w.units[ui].state = State::GettingSnowball;
            w.units[ui].counter = w.now();
            return Step::Wait;
        }
    }

    if let Some(tp) = w.units[ui].target_pos {
        if is_in_range(&w.units[ui], tp) {
            // arrived: it may throw and make snowballs again
            let u = &mut w.units[ui];
            u.run_mode = false;
            if u.stray_range == 1 {
                u.stray_range = 3;
            }
            u.last_tiles.clear();
            if w.rng.randint(1, 3) == 1 {
                let (dx, dy) = random_step(w);
                if dx == 0 && dy == 0 {
                    return Step::Fail;
                }
                return Step::Go(dx, dy);
            }
            return Step::Fail;
        }
        return find_next_tile_to_pos(w, ui, tp.0, tp.1);
    }
    // go where our penguins were hit
    let me_pos = (w.units[ui].x, w.units[ui].y);
    if let Some(p) = w.igloo_player(ui).and_then(|p| w.players.get(&p)) {
        let range = p.snowballer_alert_range;
        let mut best: Option<((i32, i32), f64)> = None;
        let mut any = false;
        for &(pos, v) in &p.alert_tiles {
            let l = dist(pos, me_pos);
            if range >= l && v != 0 {
                any = true;
                if l > 0.0 && best.map(|b| l < b.1).unwrap_or(true) {
                    best = Some((pos, l));
                }
            }
        }
        if any {
            return match best {
                Some((pos, _)) => find_next_tile_to_pos(w, ui, pos.0, pos.1),
                None => Step::Wait,
            };
        }
    }
    stray_at_igloo(w, ui, false)
}

fn worker_work(w: &mut World, ui: usize) -> Step {
    if w.units[ui].go_to_building().is_none() {
        if let Some(tp) = w.units[ui].target_pos {
            if is_in_range(&w.units[ui], tp) {
                w.units[ui].last_tiles.clear();
                let r = w.rng.randint(1, 4);
                if w.units[ui].holding.is_some() || r == 1 {
                    let (dx, mut dy) = random_step(w);
                    if dx == 0 && dy == 0 {
                        dy = 1;
                    }
                    return Step::Go(dx, dy);
                }
                return Step::Fail;
            }
            return find_next_tile_to_pos(w, ui, tp.0, tp.1);
        }
        let d = if w.units[ui].holding.is_some() {
            // carrying something: take it home
            let ig = w.units[ui].igloo;
            let d = move_to_building(w, ui, ig);
            let (dx, dy) = if let Step::Go(a, b) = d { (a, b) } else { (0, 0) };
            let next = (w.units[ui].x + dx, w.units[ui].y + dy);
            if next == (w.buildings[ig].x, w.buildings[ig].y) {
                w.units[ui].state = State::Unloading;
                w.units[ui].current_frame = 1;
                return Step::Wait;
            }
            d
        } else {
            let g = w.units[ui].going_for;
            move_to_resource(w, ui, g)
        };
        if d.is_go() {
            return d;
        }
        if matches!(w.units[ui].state, State::Gathering | State::Unloading) {
            return Step::Wait;
        }
        return stray_at_igloo(w, ui, false);
    }
    stray_at_igloo(w, ui, false)
}

fn move_to_resource(w: &mut World, ui: usize, kind: Option<ResKind>) -> Step {
    if w.units[ui].target_resource.is_none() {
        let Some(kind) = kind else { return Step::Fail };
        // pick one of the nearest resources at random
        let me = (w.units[ui].x, w.units[ui].y);
        let candidates: Vec<usize> = if kind == ResKind::Fish {
            match w.igloo_player(ui).and_then(|p| w.players.get(&p)) {
                Some(p) => p.open_resources.iter().copied().collect(),
                None => Vec::new(),
            }
        } else {
            (0..w.resources.len()).collect()
        };
        let mut best: Option<(f64, usize)> = None;
        for r in candidates {
            let res = &w.resources[r];
            if res.kind == kind && res.quantity > 0 {
                let t = &w.tiles[res.tile];
                let d = dist((t.x, t.y), me);
                match best {
                    None => best = Some((d, r)),
                    Some((bd, _)) => {
                        if d < bd + 6.0 && w.rng.randint(0, 1) == 1 {
                            best = Some((d, r));
                        }
                    }
                }
            }
        }
        match best {
            Some((_, r)) => w.units[ui].target_resource = Some(r),
            None => return Step::Fail,
        }
    }
    let r = w.units[ui].target_resource.unwrap();
    let rt = w.resources[r].tile;
    let (tx, ty) = (w.tiles[rt].x, w.tiles[rt].y);
    let d = find_next_tile_to_pos(w, ui, tx, ty);
    let Step::Go(dx, dy) = d else { return d };
    let next = w.tile_index(w.units[ui].x + dx, w.units[ui].y + dy);
    if next == Some(rt) {
        if w.resources[r].quantity > 0 {
            let now = w.now();
            let u = &mut w.units[ui];
            u.gathering_from = Some(rt);
            u.counter = now;
            u.state = State::Gathering;
            u.direction = (dx, dy);
            return Step::Wait;
        }
        // it ran out
        w.units[ui].target_resource = None;
        return Step::Fail;
    }
    d
}

/// Advance the animation (at 35 steps per second whatever the frame rate).
pub fn animate(w: &mut World, ui: usize, dt: f64) {
    w.units[ui].anim_time += dt;
    if w.units[ui].anim_time < ANIM_STEP_MS {
        return;
    }
    let steps = (w.units[ui].anim_time / ANIM_STEP_MS).floor();
    w.units[ui].anim_time -= steps * ANIM_STEP_MS;
    for _ in 0..(steps as i32).min(5) {
        animate_step(w, ui);
    }
}

fn animate_step(w: &mut World, ui: usize) {
    w.units[ui].current_frame += 1;
    if w.units[ui].current_frame > 9 {
        w.units[ui].current_frame = 0;
        if w.units[ui].state == State::Unloading {
            // finished unloading
            w.units[ui].state = State::Stray;
            if w.is_server() {
                if let Some(h) = w.units[ui].holding {
                    let ig = w.units[ui].igloo;
                    w.add_storage(ig, h, 1);
                    plugins::fire(w, Event::PenguinUnloaded(ui, h));
                }
            }
            w.units[ui].holding = None;
        }
    }
    let u = &w.units[ui];
    if u.moving && (u.current_frame == 6 || u.current_frame == 9) && w.footprints {
        let t = crate::gfx::time();
        let (mut x, mut y) = u.sprite_xy(t);
        let f6 = u.current_frame == 6;
        if u.direction.0 == 0 {
            if f6 {
                x += 8.0;
                y += 10.0;
            } else {
                x += 17.0;
                y += 10.0;
            }
        } else if u.direction.1 == 0 {
            y += 16.0;
            if f6 {
                x += 11.0;
                y += 4.0;
            } else {
                x += 9.0;
                y += 8.0;
            }
        } else {
            y += 16.0;
            if f6 {
                x += 16.0;
                y += 8.0;
            } else {
                x += 16.0;
                y += 3.0;
            }
        }
        let now = w.now();
        w.imprints.foot.add(x, y, [1.0; 3], now);
    }
}

// -- client side ---------------------------------------------------------------

/// A client's penguins follow what the server says.
fn slave_move(w: &mut World, ui: usize) {
    let start = (w.units[ui].x, w.units[ui].y);
    let mut try_next = true;
    let now = w.now();
    if w.units[ui].state == State::Throwing && w.units[ui].current_frame < 9 {
        // still throwing
        try_next = false;
    } else if w.units[ui].moving {
        let passed = now.wrapping_sub(w.units[ui].counter) as f64;
        if passed / walk_speed(w, ui) * 32.0 < 32.0 {
            try_next = false;
        }
    }
    if try_next {
        if let Some((p, dir, state, holding)) = w.units[ui].next_position.take() {
            let u = &mut w.units[ui];
            u.direction = if dir == (0, 0) { u.direction } else { dir };
            u.state = state;
            u.holding = holding;
            u.moving = (u.x, u.y) != p;
            u.x = p.0;
            u.y = p.1;
            if state == State::Throwing {
                if let Some((sx, sy, ex, ey, et)) = u.next_snowball.take() {
                    u.current_frame = 1;
                    u.moving = false;
                    let s = Snowball::new(w, sx, sy, ex, ey, et, None, None);
                    w.snowballs.push(s);
                    let u = &mut w.units[ui];
                    if u.snowballs == 0 {
                        u.next_position = Some(((u.x, u.y), u.direction, State::GettingSnowball, None));
                    }
                }
            }
            w.units[ui].counter = now;
            update_node(w, ui);
        } else {
            w.units[ui].moving = false;
        }
    }
    let end = (w.units[ui].x, w.units[ui].y);
    if start != end {
        move_sprites(w, ui, start, end);
    }
}

pub fn handle_position(w: &mut World, ui: usize, x: i32, y: i32, dir: (i32, i32), state: State, holding: Option<ResKind>) {
    w.units[ui].next_position = Some(((x, y), dir, state, holding));
}

pub fn handle_snowball(w: &mut World, ui: usize, m: &Msg) {
    if let Msg::Snowball { start_x, start_y, end_x, end_y, end_tile_x, end_tile_y, snowballs_left, px, py, .. } = m {
        let Some(et) = w.tile_index(*end_tile_x as i32, *end_tile_y as i32) else { return };
        w.units[ui].snowballs = *snowballs_left as i32;
        let dir = direction_to_tile(w, ui, *end_tile_x as i32, *end_tile_y as i32);
        w.units[ui].next_position = Some(((*px as i32, *py as i32), dir, State::Throwing, None));
        w.units[ui].next_snowball = Some((*start_x as f64, *start_y as f64, *end_x as f64, *end_y as f64, et));
    }
}

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
    (800.0 + d * 50.0) / WALK_SPEED_FACTOR
}

/// Penguins walk this much faster than in the original game (1.2: 20%):
/// a tile takes 800 ms / 1.2 on flat ground, a bit more uphill.
pub const WALK_SPEED_FACTOR: f64 = 1.2;

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
    // the door of the igloo the penguin is heading into: always allowed
    // (other penguins standing there or having walked over it a moment
    // ago used to keep cold penguins wandering around outside)
    let door = w.units[ui].go_to_building().map(|b| (w.buildings[b].x, w.buildings[b].y));
    if door == Some(pos) && w.units[ui].state != State::Entering && w.tiles[ti].resource.is_none() {
        return true;
    }
    // the next step of a path round walls: other penguins and its own
    // trail don't turn it away (a group following one path round a wall
    // used to jam and wander back and forth)
    let on_path = w.units[ui].path.first() == Some(&pos);
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
            if on_path {
                break;
            }
            // frozen penguins don't block the way (a group of them frozen
            // around an igloo door used to lock everybody out)
            if u != ui && w.units[u].state != State::Frozen && (w.units[u].x, w.units[u].y) == pos {
                return false;
            }
        }
    } else {
        w.units[ui].state = State::InBuilding;
    }
    if w.tiles[ti].resource.is_some() || crate::wall::solid(w, ti) {
        return false;
    }
    // no slipping between the ends of a diagonal wall
    if d.0 != 0 && d.1 != 0 {
        let side = |p: (i32, i32)| w.tile_index(p.0, p.1).is_some_and(|i| crate::wall::solid(w, i));
        if side((x + d.0, y)) && side((x, y + d.1)) {
            return false;
        }
    }
    let u = &mut w.units[ui];
    if u.last_tiles.len() >= 30 {
        u.last_tiles.remove(0);
    }
    on_path || !u.last_tiles.contains(&pos)
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
        if igloo::can_enter(w, ig, ui) && w.units[ui].go_to_building() != Some(ig) {
            // a new way home: forget the tiles walked so far
            w.units[ui].last_tiles.clear();
            w.units[ui].set_go_to_building(Some(ig));
        }
    } else {
        if w.units[ui].state == State::GettingSnowball {
            if w.units[ui].counter.wrapping_add(reload_ms(w, ui, cfg.create_snowball_time)) <= w.now() {
                w.units[ui].snowballs += 1;
                plugins::fire(w, Event::PenguinReload(ui));
                w.units[ui].state = State::Stray;
            } else {
                return Step::Wait;
            }
        }
        if w.units[ui].state == State::Gathering && w.units[ui].target_pos.is_none() && w.units[ui].build_wall.is_none() {
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
        } else if w.units[ui].state == State::Gathering && w.units[ui].build_wall.is_none() {
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
        // (an enemy wall in the way: knock it down)
        if kind == UnitKind::Snowballer && w.units[ui].snowballs > 0 && !w.units[ui].run_mode {
            if let Some(wi) = wall_in_the_way(w, ui) {
                throw_at_wall(w, ui, wi);
                return Step::Wait;
            }
        }
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
    if throw_log() {
        let (tx, ty) = (w.units[target].x, w.units[target].y);
        let (x, y) = (w.units[ui].x, w.units[ui].y);
        println!("THROW t={} unit={} power={:?} dist={} moving={} dx={} dy={} height={} allied={}", w.now(), ui, plugins::power_of(w, ui), (tx - x).abs().max((ty - y).abs()), w.units[target].moving, tx - x, ty - y, (w.elevation(x, y) - w.elevation(tx, ty)).round() as i32, crate::alliance::allied(w, w.igloo_player(ui), w.igloo_player(target)));
    }
    let toward = (w.units[target].x, w.units[target].y);
    launch(w, ui, s, toward);
}

/// Throw at an enemy wall section.
fn throw_at_wall(w: &mut World, ui: usize, wi: usize) {
    let toward = (w.walls[wi].x, w.walls[wi].y);
    let s = Snowball::throw_at_tile(w, ui, toward);
    if throw_log() {
        println!("THROW-WALL t={} unit={} wall={} hp={}", w.now(), ui, wi, w.walls[wi].hp);
    }
    launch(w, ui, s, toward);
}

/// The wall section a snowballer was sent to knock down, when in range.
fn ordered_wall(w: &mut World, ui: usize) -> Option<usize> {
    let wi = w.units[ui].wall_target?;
    let (x, y) = (w.units[ui].x, w.units[ui].y);
    let s = &w.walls[wi];
    if !s.built() {
        w.units[ui].wall_target = None;
        return None;
    }
    let r = plugins::throw_range(w, ui) + height_bonus(w.elevation(x, y), w.elevation(s.x, s.y)) + 0.5;
    let (dx, dy) = ((s.x - x) as f64, (s.y - y) as f64);
    (dx * dx + dy * dy <= r * r).then_some(wi)
}

/// An enemy wall section right next to a snowballer, between it and where
/// it is going (an enemy igloo, the place it was sent to).
fn wall_in_the_way(w: &World, ui: usize) -> Option<usize> {
    let u = &w.units[ui];
    let dest = match u.go_to_building() {
        Some(b) if w.buildings[b].player != w.igloo_player(ui) => (w.buildings[b].x, w.buildings[b].y),
        _ => {
            let tp = u.target_pos?;
            if is_in_range(w, u, tp) {
                return None;
            }
            tp
        }
    };
    let step = ((dest.0 - u.x).signum(), (dest.1 - u.y).signum());
    if step == (0, 0) {
        return None;
    }
    let mine = w.igloo_player(ui);
    let mut tries = vec![step];
    if step.0 != 0 && step.1 != 0 {
        tries.push((step.0, 0));
        tries.push((0, step.1));
    }
    tries.into_iter().find_map(|d| {
        let wi = crate::wall::at(w, u.x + d.0, u.y + d.1)?;
        let s = &w.walls[wi];
        (s.built() && !crate::alliance::friendly(w, s.owner, mine)).then_some(wi)
    })
}

/// A wall section for a snowballer to throw at when no penguins are in
/// range: one near its way somewhere.
fn wall_to_hit(w: &mut World, ui: usize) -> Option<usize> {
    let going = w.units[ui].target_pos.is_some() || w.units[ui].go_to_building().is_some_and(|b| w.buildings[b].player != w.igloo_player(ui));
    if going {
        crate::wall::enemy_near(w, ui, WALL_IN_THE_WAY)
    } else {
        None
    }
}

/// Snowballers on their way somewhere knock down enemy walls this close.
const WALL_IN_THE_WAY: f64 = 2.5;

fn launch(w: &mut World, ui: usize, s: Snowball, toward: (i32, i32)) {
    let by = w.igloo_player(ui);
    crate::stats::add(w, by, |st| st.thrown += 1);
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
    let (tx, ty) = toward;
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
    // a new order calls off a class change
    u.switch_pending = false;
    u.run_mode = run_mode;
    u.last_tiles.clear();
    u.set_go_to_building(None);
    u.target_pos = None;
    u.build_wall = None;
    u.wall_target = None;
    if u.state == State::Gathering {
        u.state = State::Stray;
    }
    // a wall: workers of its side work on it, snowballers of the others
    // knock it down
    if let Some(wi) = crate::wall::at(w, pos.0, pos.1) {
        if crate::wall::work_on(w, ui, wi) {
            return;
        }
        let s = &w.walls[wi];
        if w.units[ui].kind == UnitKind::Snowballer && s.built() && !crate::alliance::friendly(w, s.owner, w.igloo_player(ui)) {
            let u = &mut w.units[ui];
            u.wall_target = Some(wi);
            u.target_pos = Some(pos);
            u.stray_range = 1;
            return;
        }
    }
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
    u.build_wall = None;
    u.wall_target = None;
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
    // (walls in the way: a path round them)
    if let Some(s) = crate::path::step(w, ui, (x, y)) {
        return s;
    }
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

/// Changing class (worker <-> snowballer) costs this much fish ...
pub const SWITCH_FISH: i32 = 1;
/// ... and takes this long inside the igloo, after warming up.
pub const SWITCH_MS: u32 = 10_000;

/// Send `player`'s penguins home to change class, as many as their igloos
/// have fish for (server / single player).  Returns how many went and how
/// many didn't for lack of fish.
pub fn order_switch(w: &mut World, player: Option<Pid>, ids: &[usize]) -> (usize, usize) {
    let (mut sent, mut poor) = (0, 0);
    for &ui in ids {
        let u = &w.units[ui];
        if w.igloo_player(ui) != player || u.switch_pending || u.warmth <= 0 || matches!(u.state, State::InBuilding | State::Entering) {
            continue;
        }
        let ig = u.igloo;
        if w.buildings[ig].player != player {
            continue;
        }
        // fish not already promised to penguins on their way here
        let promised = w.buildings[ig].units.iter().filter(|&&o| w.units[o].switch_pending).count() as i32;
        if w.num_storage(ig, ResKind::Fish) - promised * SWITCH_FISH < SWITCH_FISH {
            poor += 1;
            continue;
        }
        let u = &mut w.units[ui];
        u.switch_pending = true;
        u.switch_done_at = None;
        u.target_pos = None;
        u.target_resource = None;
        u.run_mode = false;
        u.last_tiles.clear();
        if matches!(u.state, State::Gathering | State::GettingSnowball) {
            u.state = State::Stray;
            u.gathering_from = None;
        }
        u.set_go_to_building(Some(ig));
        sent += 1;
    }
    (sent, poor)
}

/// Inside the igloo: become the other class (paying for it), or not if
/// the fish is gone.  True if it changed.
pub fn finish_switch(w: &mut World, bi: usize, ui: usize) -> bool {
    let u = &mut w.units[ui];
    u.switch_pending = false;
    u.switch_done_at = None;
    if w.num_storage(bi, ResKind::Fish) < SWITCH_FISH {
        return false;
    }
    w.add_storage(bi, ResKind::Fish, -SWITCH_FISH);
    // whatever a worker carried goes into the igloo
    if let Some(h) = w.units[ui].holding.take() {
        w.add_storage(bi, h, 1);
    }
    let new = match w.units[ui].kind {
        UnitKind::Worker => UnitKind::Snowballer,
        UnitKind::Snowballer => {
            plugins::drop_power(w, ui);
            UnitKind::Worker
        }
    };
    w.set_unit_kind(ui, new);
    if new == UnitKind::Snowballer {
        w.units[ui].snowballs = 2;
    }
    let m = w.units[ui].max_warmth;
    w.set_warmth(ui, m);
    plugins::send_unit_kind(w, ui);
    true
}

/// How long making a snowball takes this penguin (rapid fire: the whole
/// make-and-throw round takes a third of the usual time).
pub fn reload_ms(w: &World, ui: usize, normal: u32) -> u32 {
    if plugins::power_of(w, ui) == Some(plugins::PowerKind::DoubleSnowball) {
        let round = normal + THROW_ROUND_MS;
        (round / plugins::RAPID_RELOAD_DIV).saturating_sub(THROW_ROUND_MS).max(200)
    } else {
        normal
    }
}

/// About what a throw adds to a make-and-throw round: the throwing
/// animation, picking a target and stepping around (measured: a normal
/// snowballer in a fight throws every ~4 s with its 3 s reload).
const THROW_ROUND_MS: u32 = 1000;

/// Test hook: print every throw (SNOWBALLZ_THROW_LOG).
pub fn throw_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SNOWBALLZ_THROW_LOG").is_ok())
}

/// Nothing to do but stand around: no enemy in range, not on the way
/// somewhere (an order, an igloo).
fn undisturbed(w: &World, ui: usize) -> bool {
    let u = &w.units[ui];
    if u.go_to_building().is_some() || u.run_mode {
        return false;
    }
    if let Some(tp) = u.target_pos {
        if !is_in_range(w, u, tp) {
            return false;
        }
    }
    enemies_in_range(w, ui).is_empty()
}

/// The closest snow tile (without anything on it) within `r` tiles.
fn snow_near(w: &World, x: i32, y: i32, r: i32) -> Option<(i32, i32)> {
    let mut best: Option<((i32, i32), i32)> = None;
    for dy in -r..=r {
        for dx in -r..=r {
            let d = dx * dx + dy * dy;
            if d == 0 || best.is_some_and(|b| d >= b.1) {
                continue;
            }
            let Some(ti) = w.tile_index(x + dx, y + dy) else { continue };
            let t = &w.tiles[ti];
            if t.kind == TileKind::Snow && t.resource.is_none() && t.building.is_none() {
                best = Some(((x + dx, y + dy), d));
            }
        }
    }
    best.map(|b| b.0)
}

/// Enemy penguins that are close enough to throw at.
/// Height difference (thrower minus target) for one more tile of range.
pub const HEIGHT_PER_TILE: f64 = 7.5;
/// The most range high ground adds, and takes away throwing uphill.
pub const HEIGHT_BONUS_MAX: f64 = 3.0;
pub const HEIGHT_MALUS_MAX: f64 = 2.0;

/// Extra tiles of range throwing from `here` down (or up) to `there`
/// (heights run 0-30: from the top of the highest hill +3 tiles).
pub fn height_bonus(here: f64, there: f64) -> f64 {
    ((here - there) / HEIGHT_PER_TILE).clamp(-HEIGHT_MALUS_MAX, HEIGHT_BONUS_MAX)
}

fn enemies_in_range(w: &World, ui: usize) -> Vec<usize> {
    let u = &w.units[ui];
    let mine = w.buildings[u.igloo].player;
    let range = plugins::throw_range(w, ui);
    let here = w.elevation(u.x, u.y);
    // the same distance every way (a circle of tiles), a bit more from
    // higher ground, a bit less uphill
    let mut out = Vec::new();
    for o in w.units_within(u.x, u.y, (range + HEIGHT_BONUS_MAX).ceil() as i32 + 1) {
        let ou = &w.units[o];
        let theirs = w.buildings[ou.igloo].player;
        if theirs != mine && !crate::alliance::allied(w, mine, theirs) && ou.warmth > 0 && ou.state != State::InBuilding {
            let r = range + height_bonus(here, w.elevation(ou.x, ou.y)) + 0.5;
            let (dx, dy) = ((ou.x - u.x) as f64, (ou.y - u.y) as f64);
            if dx * dx + dy * dy <= r * r {
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
    // an ally's snowball (thrown at somebody else) doesn't hurt
    if thrower.is_some_and(|t| crate::alliance::allied(w, w.igloo_player(t), w.igloo_player(ui))) {
        return 0;
    }
    // a direct hit takes 3 warmth, a near miss 1; with the defense
    // power-up a direct hit takes 1 and a near miss nothing
    let defended = plugins::power_of(w, ui) == Some(plugins::PowerKind::Defense);
    let mut d = 0;
    let mut hit = 0;
    if (x, y) == (ux, uy) {
        d = if defended { 1 } else { 3 };
        w.units[ui].last_tiles.clear();
        hit = 2;
        plugins::fire(w, Event::PenguinDirectHit(ui, d));
    } else if (x == ux - 1 || x == ux) && (y == uy - 1 || y == uy) {
        d = if defended { 0 } else { 1 };
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
        if let Some(by) = enemy_thrower(w, thrower, ui) {
            crate::stats::add(w, Some(by), |s| if hit == 2 { s.hits += 1 } else { s.near += 1 });
        }
    }
    if w.units[ui].warmth < 1 {
        w.set_warmth(ui, 0);
        let victim = w.igloo_player(ui);
        crate::stats::add(w, victim, |s| s.lost += 1);
        if let Some(by) = enemy_thrower(w, thrower, ui) {
            crate::stats::add(w, Some(by), |s| s.frozen += 1);
        }
        plugins::fire(w, Event::PenguinFrozen(ui, thrower));
    }
    hit
}

/// The player whose penguin threw at `ui`, when that is an enemy of its.
fn enemy_thrower(w: &World, thrower: Option<usize>, ui: usize) -> Option<Pid> {
    let by = w.igloo_player(thrower?)?;
    let victim = w.igloo_player(ui);
    (Some(by) != victim && !crate::alliance::allied(w, Some(by), victim)).then_some(by)
}

/// Has the penguin got (near enough) to where it was sent?  Not when a
/// wall stands between it and there: it is on the wrong side.
fn is_in_range(w: &World, u: &Unit, pos: (i32, i32)) -> bool {
    let r = u.stray_range;
    if pos.0 < u.x - r || pos.0 > u.x + r || pos.1 < u.y - r || pos.1 > u.y + r {
        return false;
    }
    !crate::path::wall_between(w, (u.x, u.y), pos)
}

fn dist(a: (i32, i32), b: (i32, i32)) -> f64 {
    (((a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)) as f64).sqrt()
}

fn snowballer_work(w: &mut World, ui: usize) -> Step {
    if !w.units[ui].run_mode {
        // a wall it was sent to knock down, or one right in its way, comes
        // before the penguins
        if w.units[ui].snowballs > 0 {
            if let Some(wi) = ordered_wall(w, ui).or_else(|| wall_in_the_way(w, ui)) {
                throw_at_wall(w, ui, wi);
                return Step::Wait;
            }
        }
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
        }
        // no penguins to throw at: a wall in the way?
        if w.units[ui].snowballs > 0 && w.units[ui].targeted_unit.is_none() && enemies_in_range(w, ui).is_empty() {
            if let Some(wi) = wall_to_hit(w, ui) {
                throw_at_wall(w, ui, wi);
                return Step::Wait;
            }
        }
        // make snowballs: always when out of them, and up to the number it
        // keeps (two with rapid fire) when nothing else is going on
        let n = w.units[ui].snowballs;
        if n == 0 || (n < plugins::snowballs_kept(w, ui) && undisturbed(w, ui)) {
            let (x, y) = (w.units[ui].x, w.units[ui].y);
            if w.tile(x, y).map(|t| t.kind) == Some(TileKind::Snow) {
                w.units[ui].state = State::GettingSnowball;
                w.units[ui].counter = w.now();
                return Step::Wait;
            }
            // on ice with empty flippers: step over to snow nearby
            if n == 0 && undisturbed(w, ui) {
                if let Some((sx, sy)) = snow_near(w, x, y, 6) {
                    return find_next_tile_to_pos(w, ui, sx, sy);
                }
            }
        }
    }

    if let Some(tp) = w.units[ui].target_pos {
        if is_in_range(w, &w.units[ui], tp) {
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
        // building a wall
        if let Some(s) = crate::wall::builder_step(w, ui) {
            return s;
        }
        if let Some(tp) = w.units[ui].target_pos {
            if is_in_range(w, &w.units[ui], tp) {
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
                    let by = w.igloo_player(ui);
                    crate::stats::add(w, by, |s| if h == ResKind::Crystal { s.crystal += 1 } else { s.fish += 1 });
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

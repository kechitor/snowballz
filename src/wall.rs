//! Snow walls.  Workers stack snow bricks into wall sections of
//! `SECTION_TILES` tiles (free: snow is everywhere), then can turn a
//! section into an ice wall for a crystal.
//!
//! - A brick section takes `BUILD_MS` of work: two workers on it take half
//!   as long, three a third, and so on.
//! - Turning it into ice costs `ICE_CRYSTAL` crystal and `UPGRADE_MS` of
//!   work (shared the same way).
//! - A finished section blocks the way (for everybody) and catches enemy
//!   snowballs thrown at penguins right behind it; each one that hits
//!   takes a point off it, and at none it falls apart.  Workers sent to a
//!   damaged section patch it up for free.
//!
//! The host decides everything; the players' games hear about it through
//! hidden "chat messages" (see `plugins::TIMER_FROM`).

use crate::net::Msg;
use crate::plugins::TIMER_FROM;
use crate::world::*;

/// Work (ms of one worker) a brick section (`SECTION_TILES` tiles) takes.
pub const BUILD_MS: u32 = 30_000;
/// Work turning a section into ice takes ...
pub const UPGRADE_MS: u32 = 15_000;
/// ... and the crystal it costs.
pub const ICE_CRYSTAL: i32 = 1;
/// Snowball hits a wall tile takes.
pub const BRICK_HP: i32 = 8;
pub const ICE_HP: i32 = 20;
/// Patching a section up: this much work a point.
pub const REPAIR_MS_PER_HP: u32 = 2_000;
/// A wall catches snowballs aimed at penguins up to this far behind it.
pub const COVER_TILES: i32 = 2;
/// No walls this close to an igloo door (penguins must get in and out).
pub const DOOR_CLEAR: i32 = 2;
/// The longest wall one request plans.
pub const MAX_LINE: usize = 24;
/// Tiles in a section: a section is built (and turned into ice) as one.
pub const SECTION_TILES: usize = 8;
/// Workers look for more work this far from the section they finished.
const NEXT_JOB_TILES: i32 = 6;
/// Progress is sent to the players this often at most.
const SYNC_MS: u32 = 400;

#[derive(Clone, Debug)]
pub struct Wall {
    pub x: i32,
    pub y: i32,
    /// the section this tile of wall belongs to (built and turned into ice
    /// together)
    pub section: u32,
    pub owner: Option<Pid>,
    pub ice: bool,
    /// work done on the bricks (done at `BUILD_MS`)
    pub work: u32,
    pub hp: i32,
    /// turning into ice: the work done on it so far
    pub upgrade: Option<u32>,
    /// work towards the next point patched up
    pub repair: u32,
    pub alive: bool,
    /// (host) changed since last sent; `urgent`: send right away
    pub dirty: bool,
    pub urgent: bool,
    pub sent_at: u32,
    /// when a snowball last hit it (it shakes)
    pub hit_at: Option<u32>,
}

impl Wall {
    pub fn built(&self) -> bool {
        self.alive && self.work >= BUILD_MS
    }

    pub fn max_hp(&self) -> i32 {
        if self.ice {
            ICE_HP
        } else {
            BRICK_HP
        }
    }

    pub fn needs_work(&self) -> bool {
        self.alive && (!self.built() || self.upgrade.is_some() || self.hp < self.max_hp())
    }

    /// 0..1: how far the current job (building, ice) has got.
    pub fn progress(&self) -> f64 {
        if !self.built() {
            self.work as f64 / BUILD_MS as f64
        } else if let Some(u) = self.upgrade {
            u as f64 / UPGRADE_MS as f64
        } else {
            1.0
        }
    }
}

/// Planning walls (a mode the player is in after pressing B or U).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WallMode {
    Build,
    Ice,
}

// -- queries ---------------------------------------------------------------------

pub fn at(w: &World, x: i32, y: i32) -> Option<usize> {
    w.tile(x, y).and_then(|t| t.wall).filter(|&i| w.walls[i].alive)
}

/// A finished section stands on this tile (it blocks the way).
pub fn solid(w: &World, ti: usize) -> bool {
    w.tiles[ti].wall.is_some_and(|i| w.walls[i].built())
}

/// Can a section go here?
pub fn can_place(w: &World, x: i32, y: i32) -> bool {
    let Some(t) = w.tile(x, y) else { return false };
    if t.building.is_some() || t.resource.is_some() || at(w, x, y).is_some() {
        return false;
    }
    // not on (or by) an igloo door
    !w.buildings.iter().any(|b| (b.x - x).abs() <= DOOR_CLEAR && (b.y - y).abs() <= DOOR_CLEAR)
}

/// The tiles of a straight wall from `a` towards `b` (snapped to the
/// nearest of the 8 directions).
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (ax, ay) = (dx.abs(), dy.abs());
    let (sx, sy, n) = if ax >= 2 * ay {
        (dx.signum(), 0, ax)
    } else if ay >= 2 * ax {
        (0, dy.signum(), ay)
    } else {
        (dx.signum(), dy.signum(), ax.max(ay))
    };
    (0..=n).take(MAX_LINE).map(|i| (a.0 + sx * i, a.1 + sy * i)).collect()
}

/// The ways a wall piece can lie: east-west, NW-SE, north-south, NE-SW.
pub const ROTATIONS: [(i32, i32); 4] = [(1, 0), (1, 1), (0, 1), (1, -1)];

/// The tiles of a wall piece (one section) around the tile `c`, lying the
/// `rot` way.
pub fn piece_tiles(c: (i32, i32), rot: usize) -> Vec<(i32, i32)> {
    let d = ROTATIONS[rot % ROTATIONS.len()];
    let back = (SECTION_TILES as i32 - 1) / 2;
    (-back..SECTION_TILES as i32 - back).map(|i| (c.0 + d.0 * i, c.1 + d.1 * i)).collect()
}

fn chebyshev(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// All the crystal in a player's igloos.
pub fn crystal_of(w: &World, p: Pid) -> i32 {
    (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(p)).map(|b| w.num_storage(b, ResKind::Crystal)).sum()
}

/// Take crystal from a player's igloos (the fullest first).
fn pay_crystal(w: &mut World, p: Pid, mut n: i32) {
    let mut igloos: Vec<usize> = (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(p)).collect();
    igloos.sort_by_key(|&b| -w.num_storage(b, ResKind::Crystal));
    for b in igloos {
        let take = w.num_storage(b, ResKind::Crystal).min(n);
        if take > 0 {
            w.add_storage(b, ResKind::Crystal, -take);
            n -= take;
        }
        if n == 0 {
            break;
        }
    }
}

/// The section the mouse is over on this tile (a section stands up into
/// the tile above its own, so that counts too).
pub fn pick(w: &World, (x, y): (i32, i32)) -> Option<usize> {
    at(w, x, y).or_else(|| at(w, x, y + 1))
}

/// Own brick sections on (or just under) these tiles that can be turned
/// into ice.
pub fn upgradable(w: &World, owner: Option<Pid>, tiles: &[(i32, i32)]) -> Vec<usize> {
    let mut out: Vec<usize> = tiles
        .iter()
        .filter_map(|&p| pick(w, p))
        .filter(|&i| {
            let s = &w.walls[i];
            s.owner == owner && s.built() && !s.ice && s.upgrade.is_none()
        })
        .flat_map(|i| if section_built(w, i) { mates(w, i) } else { Vec::new() })
        .filter(|&i| !w.walls[i].ice && w.walls[i].upgrade.is_none())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The (standing or planned) tiles of a tile's section.
pub fn mates(w: &World, wi: usize) -> Vec<usize> {
    let sec = w.walls[wi].section;
    (0..w.walls.len()).filter(|&i| w.walls[i].alive && w.walls[i].section == sec).collect()
}

/// Every tile of the section is up.
pub fn section_built(w: &World, wi: usize) -> bool {
    mates(w, wi).iter().all(|&i| w.walls[i].built())
}

fn new_section(w: &World) -> u32 {
    w.walls.iter().map(|s| s.section + 1).max().unwrap_or(0)
}

fn workers_on(w: &World, wi: usize) -> usize {
    w.units.iter().filter(|u| u.build_wall == Some(wi)).count()
}

// -- the host's side ----------------------------------------------------------------

fn new_wall(w: &mut World, x: i32, y: i32, owner: Option<Pid>, section: u32) -> usize {
    let wall = Wall { x, y, section, owner, ice: false, work: 0, hp: 0, upgrade: None, repair: 0, alive: true, dirty: true, urgent: true, sent_at: 0, hit_at: None };
    // (reuse a fallen section's slot)
    let i = match w.walls.iter().position(|s| !s.alive) {
        Some(i) => {
            w.walls[i] = wall;
            i
        }
        None => {
            w.walls.push(wall);
            w.walls.len() - 1
        }
    };
    let ti = w.tile_index(x, y).unwrap();
    w.tiles[ti].wall = Some(i);
    i
}

fn touch(w: &mut World, wi: usize, urgent: bool) {
    w.walls[wi].dirty = true;
    w.walls[wi].urgent |= urgent;
}

/// Send `workers` to work on `walls`: each goes to the nearest section
/// with the fewest workers on it.
fn assign(w: &mut World, workers: &[usize], walls: &[usize]) {
    if walls.is_empty() {
        return;
    }
    let mut count: Vec<usize> = walls.iter().map(|&i| workers_on(w, i)).collect();
    for &ui in workers {
        let me = (w.units[ui].x, w.units[ui].y);
        let best = (0..walls.len())
            .min_by_key(|&k| {
                let s = &w.walls[walls[k]];
                (count[k], chebyshev(me, (s.x, s.y)))
            })
            .unwrap();
        count[best] += 1;
        set_builder(w, ui, Some(walls[best]));
    }
}

fn set_builder(w: &mut World, ui: usize, wi: Option<usize>) {
    let u = &mut w.units[ui];
    u.switch_pending = false;
    u.target_pos = None;
    u.target_resource = None;
    u.gathering_from = None;
    u.last_tiles.clear();
    u.set_go_to_building(None);
    if u.state == State::Gathering {
        u.state = State::Stray;
    }
    u.build_wall = wi;
}

/// Workers from the list that can be given a wall job by `owner`.
fn own_workers(w: &World, owner: Option<Pid>, ids: &[usize]) -> Vec<usize> {
    ids.iter()
        .copied()
        .filter(|&u| u < w.units.len())
        .filter(|&u| w.units[u].kind == UnitKind::Worker && w.igloo_player(u) == owner && owner.is_some())
        .filter(|&u| !matches!(w.units[u].state, State::Entering | State::InBuilding | State::Frozen))
        .collect()
}

/// Plan a wall from `a` to `b` and send the workers to build it.
pub fn plan(w: &mut World, owner: Option<Pid>, a: (i32, i32), b: (i32, i32), ids: &[usize]) -> usize {
    let workers = own_workers(w, owner, ids);
    if workers.is_empty() {
        return 0;
    }
    // (every `SECTION_TILES` tiles make a section)
    let tiles: Vec<(i32, i32)> = line(a, b).into_iter().filter(|&(x, y)| can_place(w, x, y)).collect();
    let mut new = Vec::new();
    let mut sec = new_section(w);
    for (k, (x, y)) in tiles.into_iter().enumerate() {
        if k > 0 && k % SECTION_TILES == 0 {
            sec += 1;
        }
        new.push(new_wall(w, x, y, owner, sec));
    }
    assign(w, &workers, &new);
    new.len()
}

/// Turn own brick sections into ice (a crystal each, as far as the
/// crystal goes) and send the workers to do it.  Returns (started, the
/// ones there was no crystal for).
pub fn upgrade(w: &mut World, owner: Option<Pid>, walls: &[usize], ids: &[usize]) -> (usize, usize) {
    let workers = own_workers(w, owner, ids);
    let Some(p) = owner else { return (0, 0) };
    if workers.is_empty() {
        return (0, 0);
    }
    // whole sections, a crystal each
    let mut started = Vec::new();
    let mut done_sections = Vec::new();
    let (mut n, mut poor) = (0, 0);
    for &wi in walls {
        let s = &w.walls[wi];
        if !(s.alive && s.owner == owner && s.built() && !s.ice && s.upgrade.is_none()) || done_sections.contains(&s.section) || !section_built(w, wi) {
            continue;
        }
        done_sections.push(s.section);
        if crystal_of(w, p) < ICE_CRYSTAL {
            poor += 1;
            continue;
        }
        pay_crystal(w, p, ICE_CRYSTAL);
        n += 1;
        for m in mates(w, wi) {
            if !w.walls[m].ice && w.walls[m].upgrade.is_none() {
                w.walls[m].upgrade = Some(0);
                touch(w, m, true);
                started.push(m);
            }
        }
    }
    assign(w, &workers, &started);
    (n, poor)
}

/// A worker sent onto a section of its side's wall (a right click on it):
/// it works on it (build, ice, patch up).
pub fn work_on(w: &mut World, ui: usize, wi: usize) -> bool {
    let s = &w.walls[wi];
    if !s.alive || w.units[ui].kind != UnitKind::Worker || !crate::alliance::friendly(w, s.owner, w.igloo_player(ui)) {
        return false;
    }
    set_builder(w, ui, Some(wi));
    true
}

/// Some more work near the section a worker just finished.
fn next_job(w: &World, ui: usize, near: (i32, i32)) -> Option<usize> {
    let mine = w.igloo_player(ui);
    let me = (w.units[ui].x, w.units[ui].y);
    (0..w.walls.len())
        .filter(|&i| {
            let s = &w.walls[i];
            s.needs_work() && s.owner == mine && chebyshev((s.x, s.y), near) <= NEXT_JOB_TILES
        })
        .min_by_key(|&i| {
            let s = &w.walls[i];
            (workers_on(w, i), chebyshev(me, (s.x, s.y)))
        })
}

/// A worker with a wall job (host): `None` when it has none (any more).
pub fn builder_step(w: &mut World, ui: usize) -> Option<Step> {
    let wi = w.units[ui].build_wall?;
    let mine = w.igloo_player(ui);
    let s = &w.walls[wi];
    let pos = (s.x, s.y);
    if !s.needs_work() || !crate::alliance::friendly(w, s.owner, mine) {
        let next = next_job(w, ui, pos);
        w.units[ui].build_wall = next;
        if w.units[ui].state == State::Gathering {
            w.units[ui].state = State::Stray;
        }
        next?;
        return Some(Step::Wait);
    }
    let me = (w.units[ui].x, w.units[ui].y);
    let now = w.now();
    if me == pos {
        // standing where the bricks go: step aside
        w.units[ui].state = State::Stray;
        for d in [(0, 1), (1, 1), (-1, 1), (1, 0), (-1, 0), (0, -1), (1, -1), (-1, -1)] {
            let p = (me.0 + d.0, me.1 + d.1);
            if w.tile(p.0, p.1).is_some_and(|t| t.resource.is_none() && t.building.is_none()) && at(w, p.0, p.1).is_none() {
                return Some(Step::Go(d.0, d.1));
            }
        }
        return Some(Step::Go(0, 1));
    }
    if chebyshev(me, pos) == 1 {
        let u = &mut w.units[ui];
        if u.state != State::Gathering {
            // start: face the wall, packing snow
            u.state = State::Gathering;
            u.direction = (pos.0 - me.0, pos.1 - me.1);
            u.counter = now;
            u.last_tiles.clear();
            return Some(Step::Wait);
        }
        let dt = now.wrapping_sub(u.counter).min(1000);
        u.counter = now;
        work(w, wi, dt);
        return Some(Step::Wait);
    }
    if w.units[ui].state == State::Gathering {
        w.units[ui].state = State::Stray;
    }
    // walk over (go round the side if it is in the way)
    Some(crate::unit::find_next_tile_to_pos(w, ui, pos.0, pos.1))
}

/// Some work done on a wall tile (one worker, `dt` ms): building and
/// turning into ice go for its whole section, patching up for the tile.
fn work(w: &mut World, wi: usize, dt: u32) {
    if !w.walls[wi].built() {
        for m in mates(w, wi) {
            let s = &mut w.walls[m];
            if s.built() {
                continue;
            }
            s.work = (s.work + dt).min(BUILD_MS);
            let mut urgent = false;
            if s.work >= BUILD_MS {
                // done, as soon as nobody stands on it
                let (x, y) = (s.x, s.y);
                let blocked = w.units_around(x, y).iter().any(|&u| (w.units[u].x, w.units[u].y) == (x, y) && w.units[u].state != State::InBuilding);
                let s = &mut w.walls[m];
                if blocked {
                    s.work = BUILD_MS - 1;
                } else {
                    s.hp = BRICK_HP;
                    urgent = true;
                    w.wall_version = w.wall_version.wrapping_add(1);
                    let owner = s.owner;
                    crate::stats::add(w, owner, |st| st.walls += 1);
                }
            }
            touch(w, m, urgent);
        }
        return;
    }
    if w.walls[wi].upgrade.is_some() {
        for m in mates(w, wi) {
            let s = &mut w.walls[m];
            let Some(u) = s.upgrade else { continue };
            let u = u + dt;
            let mut urgent = false;
            if u >= UPGRADE_MS {
                s.upgrade = None;
                s.ice = true;
                s.hp = ICE_HP;
                urgent = true;
            } else {
                s.upgrade = Some(u);
            }
            touch(w, m, urgent);
        }
        return;
    }
    let s = &mut w.walls[wi];
    let urgent = false;
    if s.hp < s.max_hp() {
        s.repair += dt;
        while s.repair >= REPAIR_MS_PER_HP && s.hp < s.max_hp() {
            s.repair -= REPAIR_MS_PER_HP;
            s.hp += 1;
        }
        if s.hp >= s.max_hp() {
            s.repair = 0;
        }
    }
    touch(w, wi, urgent);
}

/// A snowball hit a section (host).
pub fn damage(w: &mut World, wi: usize, n: i32) {
    let now = w.now();
    let s = &mut w.walls[wi];
    if !s.built() {
        return;
    }
    s.hp -= n;
    s.hit_at = Some(now);
    s.repair = 0;
    if s.hp <= 0 {
        remove(w, wi);
    }
    touch(w, wi, true);
}

/// A section falls apart (knocked down, or taken down by its side).
fn remove(w: &mut World, wi: usize) {
    w.wall_version = w.wall_version.wrapping_add(1);
    let s = &mut w.walls[wi];
    s.alive = false;
    s.upgrade = None;
    let (x, y) = (s.x, s.y);
    let ti = w.tile_index(x, y).unwrap();
    if w.tiles[ti].wall == Some(wi) {
        w.tiles[ti].wall = None;
    }
    // (its builders stop)
    for u in w.units.iter_mut() {
        if u.build_wall == Some(wi) {
            u.build_wall = None;
            if u.state == State::Gathering {
                u.state = State::Stray;
            }
        }
        if u.wall_target == Some(wi) {
            u.wall_target = None;
        }
    }
    touch(w, wi, true);
}

/// Take down one of `owner`'s own sections (planned or standing).
pub fn take_down(w: &mut World, owner: Option<Pid>, wi: usize) -> bool {
    if wi >= w.walls.len() || !w.walls[wi].alive || owner.is_none() || w.walls[wi].owner != owner {
        return false;
    }
    remove(w, wi);
    true
}

/// The section (if any) that catches a snowball thrown from `from` at
/// `to` by `thrower`'s side: one of the other sides' finished sections on
/// the way, up to `COVER_TILES` before the target.
pub fn cover(w: &World, from: (i32, i32), to: (i32, i32), thrower: Option<Pid>) -> Option<usize> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let n = dx.abs().max(dy.abs());
    for i in 1..=n {
        let p = (from.0 + (dx as f64 * i as f64 / n as f64).round() as i32, from.1 + (dy as f64 * i as f64 / n as f64).round() as i32);
        if chebyshev(p, to) > COVER_TILES {
            continue;
        }
        if let Some(wi) = at(w, p.0, p.1) {
            let s = &w.walls[wi];
            if s.built() && !crate::alliance::friendly(w, s.owner, thrower) {
                return Some(wi);
            }
        }
    }
    None
}

/// An enemy section a snowballer could knock down (within `range`).
pub fn enemy_near(w: &World, ui: usize, range: f64) -> Option<usize> {
    let u = &w.units[ui];
    let mine = w.igloo_player(ui);
    let r = range.ceil() as i32;
    let mut best: Option<(i32, usize)> = None;
    for y in u.y - r..=u.y + r {
        for x in u.x - r..=u.x + r {
            if let Some(wi) = at(w, x, y) {
                let s = &w.walls[wi];
                let (ddx, ddy) = ((x - u.x) as f64, (y - u.y) as f64);
                if s.built() && !crate::alliance::friendly(w, s.owner, mine) && ddx * ddx + ddy * ddy <= range * range {
                    let d = chebyshev((x, y), (u.x, u.y));
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, wi));
                    }
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// Send changes to the players (host, every frame).
pub fn tick(w: &mut World) {
    let now = w.now();
    for wi in 0..w.walls.len() {
        let s = &w.walls[wi];
        if s.dirty && (s.urgent || now.wrapping_sub(s.sent_at) >= SYNC_MS) {
            let s = &mut w.walls[wi];
            s.dirty = false;
            s.urgent = false;
            s.sent_at = now;
            if w.role == Role::Server {
                let m = state_message(w, wi);
                w.send(m);
            }
        }
    }
}

// -- messages ------------------------------------------------------------------------

const STATE_PREFIX: &str = "#wall ";
const PLAN_PREFIX: &str = "#wall-plan ";
const ICE_PREFIX: &str = "#wall-ice ";
const REMOVE_PREFIX: &str = "#wall-remove ";

fn hidden(msg: String) -> Msg {
    Msg::ChatMessage { msg, from_id: TIMER_FROM, to_id: 0xffff }
}

fn state_message(w: &World, wi: usize) -> Msg {
    let s = &w.walls[wi];
    let owner = s.owner.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
    let up = s.upgrade.map(|u| u.to_string()).unwrap_or_else(|| "-".into());
    hidden(format!("{}{} {} {} {} {} {} {} {} {} {} {}", STATE_PREFIX, wi, s.x, s.y, owner, s.ice as u8, s.work, s.hp, up, s.alive as u8, s.hit_at.map(|t| (w.now().wrapping_sub(t) < 500) as u8).unwrap_or(0), s.section))
}

/// Host: every standing section, for a player who just joined.
pub fn send_state_to(w: &mut World, c: crate::net::ConnId) {
    for wi in 0..w.walls.len() {
        if w.walls[wi].alive {
            let m = state_message(w, wi);
            w.send_to(m, &[c], &[]);
        }
    }
}

fn ids_text(ids: &[usize]) -> String {
    ids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",")
}

fn parse_ids(s: &str) -> Vec<usize> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

/// Plan a wall (single player and host: right away; a player: asks the
/// host).
pub fn request_plan(w: &mut World, a: (i32, i32), b: (i32, i32), ids: &[usize]) {
    if w.is_server() {
        let me = w.me;
        plan(w, me, a, b, ids);
    } else {
        let m = hidden(format!("{}{} {} {} {} {}", PLAN_PREFIX, a.0, a.1, b.0, b.1, ids_text(ids)));
        w.send(m);
    }
}

pub fn request_ice(w: &mut World, walls: &[usize], ids: &[usize]) {
    if w.is_server() {
        let me = w.me;
        upgrade(w, me, walls, ids);
    } else {
        let m = hidden(format!("{}{} {}", ICE_PREFIX, ids_text(walls), ids_text(ids)));
        w.send(m);
    }
}

/// Take down one of my sections.
pub fn request_take_down(w: &mut World, wi: usize) {
    if w.is_server() {
        let me = w.me;
        take_down(w, me, wi);
    } else {
        let m = hidden(format!("{}{}", REMOVE_PREFIX, wi));
        w.send(m);
    }
}

/// Host: a player's request (true if it was one).
pub fn host_message(w: &mut World, msg: &str, from: Option<Pid>) -> bool {
    if let Some(rest) = msg.strip_prefix(REMOVE_PREFIX) {
        if let Ok(wi) = rest.trim().parse::<usize>() {
            take_down(w, from, wi);
        }
        return true;
    }
    if let Some(rest) = msg.strip_prefix(PLAN_PREFIX) {
        let v: Vec<&str> = rest.split(' ').collect();
        if v.len() == 5 {
            let n: Vec<i32> = v[..4].iter().filter_map(|s| s.parse().ok()).collect();
            if n.len() == 4 && from.is_some() {
                plan(w, from, (n[0], n[1]), (n[2], n[3]), &parse_ids(v[4]));
            }
        }
        return true;
    }
    if let Some(rest) = msg.strip_prefix(ICE_PREFIX) {
        let mut it = rest.split(' ');
        let walls: Vec<usize> = it.next().map(parse_ids).unwrap_or_default().into_iter().filter(|&i| i < w.walls.len()).collect();
        let ids = it.next().map(parse_ids).unwrap_or_default();
        if from.is_some() {
            upgrade(w, from, &walls, &ids);
        }
        return true;
    }
    false
}

/// A player's game heard from the host (true if it was about walls).
pub fn client_message(w: &mut World, msg: &str) -> bool {
    let Some(rest) = msg.strip_prefix(STATE_PREFIX) else { return false };
    let v: Vec<&str> = rest.split(' ').collect();
    if v.len() < 10 {
        return true;
    }
    let num = |i: usize| v[i].parse::<i64>().ok();
    let (Some(wi), Some(x), Some(y)) = (num(0), num(1), num(2)) else { return true };
    let (wi, x, y) = (wi as usize, x as i32, y as i32);
    let Some(ti) = w.tile_index(x, y) else { return true };
    if wi > 4096 {
        return true;
    }
    while w.walls.len() <= wi {
        w.walls.push(Wall { x: 0, y: 0, section: 0, owner: None, ice: false, work: 0, hp: 0, upgrade: None, repair: 0, alive: false, dirty: false, urgent: false, sent_at: 0, hit_at: None });
    }
    // (moved: forget the old tile)
    let old = &w.walls[wi];
    if old.alive && (old.x, old.y) != (x, y) {
        if let Some(oti) = w.tile_index(old.x, old.y) {
            if w.tiles[oti].wall == Some(wi) {
                w.tiles[oti].wall = None;
            }
        }
    }
    let now = w.now();
    let s = &mut w.walls[wi];
    s.x = x;
    s.y = y;
    s.owner = v[3].parse().ok();
    s.ice = v[4] == "1";
    s.work = num(5).unwrap_or(0) as u32;
    s.hp = num(6).unwrap_or(0) as i32;
    s.upgrade = v[7].parse().ok();
    s.alive = v[8] == "1";
    if v[9] == "1" {
        s.hit_at = Some(now);
    }
    s.section = v.get(10).and_then(|x| x.parse().ok()).unwrap_or(wi as u32);
    let alive = s.alive;
    if alive {
        w.tiles[ti].wall = Some(wi);
    } else if w.tiles[ti].wall == Some(wi) {
        w.tiles[ti].wall = None;
    }
    true
}

/// (test hook) Finished wall tiles for `owner` (one section each 8).
pub fn test_build(w: &mut World, owner: Option<Pid>, tiles: &[(i32, i32)]) -> usize {
    let mut n = 0;
    let base = new_section(w);
    for (k, &(x, y)) in tiles.iter().enumerate() {
        if can_place(w, x, y) {
            let wi = new_wall(w, x, y, owner, base + (k / SECTION_TILES) as u32);
            w.walls[wi].work = BUILD_MS;
            w.walls[wi].hp = BRICK_HP;
            n += 1;
        }
    }
    w.wall_version = w.wall_version.wrapping_add(1);
    n
}

/// (test hook, a player) My workers asked to build a wall below my igloo.
pub fn test_request(w: &mut World) {
    let me = w.me;
    let Some((x, y)) = w.buildings.iter().find(|b| b.player.is_some() && b.player == me).map(|b| (b.x, b.y)) else { return };
    let workers: Vec<usize> = (0..w.units.len()).filter(|&u| w.igloo_player(u) == me && w.units[u].kind == UnitKind::Worker).collect();
    println!("WALLTEST request {} workers, wall at {:?}", workers.len(), (x - 2, y + 6));
    request_plan(w, (x - 2, y + 6), (x + 4, y + 6), &workers);
    crate::ctrl::look_at(w, (x * 32) as f64, (y * 32 + 160) as f64);
}

/// (test hook) Crystal for me, finished walls all round the
/// other igloos, and my snowballers sent at the nearest of them.
pub fn test_setup(w: &mut World) {
    let me = w.me;
    if let Some(b) = (0..w.buildings.len()).find(|&b| w.buildings[b].player == me) {
        w.add_storage(b, ResKind::Crystal, 5);
    }
    let mine = me.and_then(|p| w.buildings.iter().find(|b| b.player == Some(p)).map(|b| (b.x, b.y))).unwrap_or((0, 0));
    let mut nearest: Option<(i32, usize)> = None;
    for bi in 0..w.buildings.len() {
        let (owner, x, y) = (w.buildings[bi].player, w.buildings[bi].x, w.buildings[bi].y);
        if owner.is_none() || owner == me {
            continue;
        }
        let d = chebyshev((x, y), mine);
        if nearest.is_none_or(|n| d < n.0) {
            nearest = Some((d, bi));
        }
        // a ring 4 tiles out from the igloo (it is 4 x 4, its door top left)
        let (cx, cy) = (x + 2, y + 2);
        let r = 6;
        for i in -r..=r {
            for (px, py) in [(cx + i, cy - r), (cx + i, cy + r), (cx - r, cy + i), (cx + r, cy + i)] {
                if can_place(w, px, py) {
                    let sec = new_section(w);
                    let wi = new_wall(w, px, py, owner, sec);
                    w.walls[wi].work = BUILD_MS;
                    w.walls[wi].hp = BRICK_HP;
                }
            }
        }
    }
    if let Some((_, bi)) = nearest {
        let door = (w.buildings[bi].x, w.buildings[bi].y);
        let sb: Vec<usize> = (0..w.units.len()).filter(|&u| w.igloo_player(u) == me && w.units[u].kind == UnitKind::Snowballer).collect();
        let n = sb.len();
        for u in sb {
            crate::unit::set_job(w, u, door, false, n);
        }
        crate::ctrl::look_at(w, (door.0 * 32) as f64, (door.1 * 32) as f64);
        println!("WALLTEST target igloo {} at {:?}, mine at {:?}, walls {}", bi, door, mine, w.walls.iter().filter(|s| s.alive).count());
    }
}

// -- drawing ---------------------------------------------------------------------------

const CELL: i32 = 64;
/// (the sheet's rows)
const ROW_BRICK_FULL: i32 = 2;
const ROW_BRICK_DAMAGED: i32 = 3;
const ROW_ICE: i32 = 4;
const ROW_ICE_DAMAGED: i32 = 5;
const ROWS: i32 = 6;
/// the arms' columns: towards E, SE, S, SW, W, NW, N, NE (8 more: the
/// same reaching a little past the middle, for corners)
const ARMS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

fn piece(row: i32, col: i32) -> crate::gfx::Tex {
    // (y counted from the bottom; textures are padded at the top)
    let sheet = crate::gfx::texture_sized("data/walls.png", 1024);
    sheet.sub(col * CELL, (ROWS - 1 - row) * CELL, CELL, CELL)
}

/// The middle of a tile's ground on the screen.
fn ground(w: &World, x: i32, y: i32) -> (f64, f64) {
    ((x * 32 + 16) as f64, (y * 32 + 22) as f64 - w.elevation(x, y) * ELEV_MULT)
}

/// Queue one wall tile's pieces: an arm towards each wall tile next to it
/// (a lone tile: a short stretch east-west).  The arms of a straight run
/// meet exactly in the middle; at corners and ends of joins they reach a
/// little past it, so the corner is closed.
fn push_section(w: &mut World, x: i32, y: i32, row: i32, color: [f32; 4], links: &dyn Fn((i32, i32)) -> bool, shake: f64) {
    let (cx, cy) = ground(w, x, y);
    let (dx0, dy0) = (cx - 32.0 + shake, cy - 40.0);
    let key = (1, cy + 10.0 - 0.5, -cx);
    let mut arms: Vec<usize> = Vec::new();
    for (i, d) in ARMS.iter().enumerate() {
        if !links((x + d.0, y + d.1)) {
            continue;
        }
        // (no diagonal where a straight side gets there)
        if d.0 != 0 && d.1 != 0 && (links((x + d.0, y)) || links((x, y + d.1))) {
            continue;
        }
        arms.push(i);
    }
    if arms.is_empty() {
        arms = vec![0, 4];
    }
    let straight = arms.len() == 1 || (arms.len() == 2 && arms[1] == arms[0] + 4);
    let extra = if straight { 0 } else { 8 };
    // the far arms first, then the near ones
    arms.sort_by_key(|&i| ARMS[i].1);
    for i in arms {
        let item = DrawItem::Tex { tex: piece(row, i as i32 + extra), x: dx0, y: dy0, color };
        w.draw_buffer.push(Rendered { key, item });
    }
}

/// Queue the walls in view (and the one being planned) for drawing.
pub fn draw(w: &mut World, view: (f64, f64, f64, f64)) {
    let (vx, vy, vw, vh) = view;
    let now = w.now();
    for wi in 0..w.walls.len() {
        let s = w.walls[wi].clone();
        if !s.alive {
            continue;
        }
        let (cx, cy) = ground(w, s.x, s.y);
        if cx + 48.0 < vx || cx - 48.0 > vx + vw || cy + 48.0 < vy || cy - 48.0 > vy + vh {
            continue;
        }
        let walls = &w.walls;
        let tiles = &w.tiles;
        let size = w.size;
        let link = move |p: (i32, i32)| -> bool {
            if p.0 < 0 || p.1 < 0 || p.0 >= size.0 || p.1 >= size.1 {
                return false;
            }
            tiles[(p.1 * size.0 + p.0) as usize].wall.is_some_and(|i| walls[i].alive)
        };
        let links: Vec<bool> = ARMS.iter().map(|d| link((s.x + d.0, s.y + d.1))).collect();
        let around = |p: (i32, i32)| ARMS.iter().position(|d| (s.x + d.0, s.y + d.1) == p).is_some_and(|i| links[i]);
        let shake = match s.hit_at {
            Some(t) if now.wrapping_sub(t) < 240 => if (now / 40).is_multiple_of(2) { 1.0 } else { -1.0 },
            _ => 0.0,
        };
        let oc = w.glcolor(s.owner);
        if !s.built() {
            if s.work == 0 {
                // planned: a faint outline in its side's colour
                push_section(w, s.x, s.y, ROW_BRICK_FULL, [oc[0], oc[1], oc[2], 0.38], &around, 0.0);
            } else {
                let rows = ((s.progress() * 3.0).ceil() as i32).clamp(1, 3);
                // the rest still to come, faintly
                if rows < 3 {
                    push_section(w, s.x, s.y, ROW_BRICK_FULL, [oc[0], oc[1], oc[2], 0.25], &around, 0.0);
                }
                push_section(w, s.x, s.y, rows - 1, [1.0; 4], &around, 0.0);
            }
            continue;
        }
        let half = s.hp * 2 <= s.max_hp();
        let row = match (s.ice, half) {
            (true, false) => ROW_ICE,
            (true, true) => ROW_ICE_DAMAGED,
            (false, false) => ROW_BRICK_FULL,
            (false, true) => ROW_BRICK_DAMAGED,
        };
        push_section(w, s.x, s.y, row, [1.0; 4], &around, shake);
        if let Some(u) = s.upgrade {
            // the ice coming up over the bricks
            let a = 0.2 + 0.7 * (u as f32 / UPGRADE_MS as f32);
            push_section(w, s.x, s.y, ROW_ICE, [1.0, 1.0, 1.0, a], &around, shake);
        }
        // whose it is: a little flag at the wall's ends
        if links.iter().filter(|&&l| l).count() <= 1 {
            let key = (1, cy + 10.0 - 0.4, -cx);
            let item = DrawItem::Tex { tex: w.tex.frozen_flag, x: cx - 4.0 + shake, y: cy - 34.0, color: [oc[0], oc[1], oc[2], 1.0] };
            w.draw_buffer.push(Rendered { key, item });
        }
    }
    draw_plan(w);
}

/// The tile under the mouse.
fn mouse_tile(w: &World) -> (i32, i32) {
    let (x, y) = crate::gfx::mouse_pos();
    w.canvas_to_map(x as f64 / w.view.zoom, y as f64 / w.view.zoom)
}

/// Planning (B / U): the wall piece under the mouse (green where it fits,
/// red where it doesn't), or the section that would turn into ice.
fn draw_plan(w: &mut World) {
    let Some(mode) = w.wall_mode else { return };
    let c = mouse_tile(w);
    let me = w.me;
    match mode {
        WallMode::Build => {
            let tiles = piece_tiles(c, w.wall_rot);
            let fits = tiles.iter().all(|&(x, y)| can_place(w, x, y));
            for &(x, y) in &tiles {
                let col = if fits {
                    [0.55, 1.0, 0.6, 0.6]
                } else if can_place(w, x, y) {
                    [1.0, 0.55, 0.45, 0.45]
                } else {
                    [1.0, 0.2, 0.15, 0.7]
                };
                let l: Vec<(i32, i32)> = ARMS.iter().map(|d| (x + d.0, y + d.1)).filter(|p| tiles.contains(p)).collect();
                push_section(w, x, y, ROW_BRICK_FULL, col, &|p| l.contains(&p), 0.0);
            }
        }
        WallMode::Ice => {
            for wi in upgradable(w, me, &[c]) {
                let (x, y) = (w.walls[wi].x, w.walls[wi].y);
                let l: Vec<(i32, i32)> = ARMS.iter().map(|d| (x + d.0, y + d.1)).filter(|&p| at(w, p.0, p.1).is_some()).collect();
                push_section(w, x, y, ROW_ICE, [0.7, 0.95, 1.0, 0.75], &|p| l.contains(&p), 0.0);
            }
        }
    }
}

/// Progress and damage bars over the walls (after the sprites).
pub fn draw_bars(w: &World, view: (f64, f64, f64, f64)) {
    let (vx, vy, vw, vh) = view;
    for (wi, s) in w.walls.iter().enumerate() {
        if !s.alive {
            continue;
        }
        let (cx, cy) = ground(w, s.x, s.y);
        if cx + 32.0 < vx || cx - 32.0 > vx + vw || cy + 32.0 < vy || cy - 32.0 > vy + vh {
            continue;
        }
        // building / turning into ice: one bar a section (over its middle)
        let middle = || {
            let mut m = mates(w, wi);
            m.sort_by_key(|&i| (w.walls[i].x, w.walls[i].y));
            m.get(m.len() / 2) == Some(&wi)
        };
        let (frac, color, bw) = if !s.built() {
            if s.work == 0 || !middle() {
                continue;
            }
            (s.progress(), [0.95, 0.97, 1.0, 1.0], 40.0)
        } else if s.upgrade.is_some() {
            if !middle() {
                continue;
            }
            (s.progress(), [0.45, 0.85, 1.0, 1.0], 40.0)
        } else if s.hp < s.max_hp() {
            let f = s.hp as f64 / s.max_hp() as f64;
            (f, [(1.0 - f as f32) * 1.6, f as f32 * 1.4, 0.15, 1.0], 20.0)
        } else {
            continue;
        };
        let (bx, by) = (cx - bw / 2.0, cy - 30.0);
        crate::gfx::fill_rect(bx - 1.0, by - 1.0, bw + 2.0, 5.0, Some([0.07, 0.09, 0.16, 0.8]));
        crate::gfx::fill_rect(bx, by, bw * frac.clamp(0.0, 1.0), 3.0, Some(color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_is_one_section_whichever_way_it_lies() {
        for rot in 0..ROTATIONS.len() {
            let t = piece_tiles((10, 10), rot);
            assert_eq!(t.len(), SECTION_TILES);
            assert!(t.contains(&(10, 10)));
            // (and it is one straight line from end to end)
            assert_eq!(line(t[0], t[t.len() - 1]), t);
        }
        assert_eq!(piece_tiles((0, 0), 0)[0], (-3, 0));
        assert_eq!(piece_tiles((0, 0), 5), piece_tiles((0, 0), 1));
    }

    #[test]
    fn lines_snap_to_eight_directions() {
        assert_eq!(line((0, 0), (5, 1)), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0)]);
        assert_eq!(line((0, 0), (1, -4)), vec![(0, 0), (0, -1), (0, -2), (0, -3), (0, -4)]);
        assert_eq!(line((2, 2), (5, 4)), vec![(2, 2), (3, 3), (4, 4), (5, 5)]);
        assert_eq!(line((0, 0), (0, 0)), vec![(0, 0)]);
        assert_eq!(line((0, 0), (40, 0)).len(), MAX_LINE);
    }
}

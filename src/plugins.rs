//! Events and the map plugins (events.py and plugins/*): power-ups on the
//! map, "conquer all igloos" and "gather N loads" victories.

use crate::world::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // some event data is only informative (as in events.py)
pub enum Event {
    PenguinMove(usize, i32, i32),
    PenguinReload(usize),
    PenguinDirectHit(usize, i32),
    /// (frozen penguin, the penguin that threw the snowball)
    PenguinFrozen(usize, Option<usize>),
    PenguinGathered(usize),
    PenguinUnloaded(usize, ResKind),
    /// (region, its previous owner)
    RegionTakeover(usize, Option<Pid>),
}

#[derive(Default)]
pub struct Plugins {
    conquer: Option<Conquer>,
    gather: Option<Gather>,
    powerup: Option<PowerupPlugin>,
}

struct Conquer {
    /// None = all igloos
    to_win: Option<i32>,
    #[allow(dead_code)]
    to_lose: i32,
}

struct Gather {
    kind: ResKind,
    amount: i32,
}

/// The power-ups.  Rapid fire ("DoubleSnowball" in map files): the
/// snowballer makes snowballs three times as fast and keeps two in hand
/// when it isn't busy.  Range: it throws further, and its snowballs
/// always find their target.  Defense: 4 more warmth, near misses don't
/// hurt, direct hits take 1 warmth instead of 3, and it warms up by
/// itself (1 warmth every 5 seconds).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PowerKind {
    DoubleSnowball,
    Range,
    Defense,
}

pub const ALL_POWERS: [PowerKind; 3] = [PowerKind::DoubleSnowball, PowerKind::Range, PowerKind::Defense];

/// Extra (and most) warmth with the defense power-up.
pub const DEFENSE_WARMTH: i32 = 4;
/// ... and it warms its carrier up by itself: 1 warmth this often (ms).
pub const DEFENSE_WARM_EVERY_MS: u32 = 15_000;
/// New power-ups are dropped this often (ms of game time).
pub const DROP_EVERY_MS: u32 = 60_000;

/// How far a snowballer throws (tiles, the same in every direction).
pub const THROW_RANGE: f64 = 7.0;
/// ... with the range power-up.
pub const THROW_RANGE_POWER: f64 = 13.0;
/// Making a snowball with the rapid-fire power-up takes this share of the
/// usual time (with the throw itself: about three times the throws).
pub const RAPID_RELOAD_DIV: u32 = 3;

impl PowerKind {
    /// The kind from its name in a map file ("Defense" became "Range").
    pub fn from_name(n: &str) -> Option<PowerKind> {
        match n {
            "DoubleSnowball" => Some(PowerKind::DoubleSnowball),
            "Range" => Some(PowerKind::Range),
            "Defense" => Some(PowerKind::Defense),
            _ => None,
        }
    }
    fn image(&self) -> &'static str {
        match self {
            PowerKind::DoubleSnowball => "plugins/powerup/powerup_1.png",
            PowerKind::Range => "plugins/powerup/powerup_3.png",
            PowerKind::Defense => "plugins/powerup/powerup_2.png",
        }
    }
    fn emblem(&self) -> Emblem {
        Emblem::new(self.image(), true, (0, -26))
    }
}

struct Powerup {
    kind: PowerKind,
    position: (i32, i32),
    /// its picture on the map (a new one each time it lands somewhere)
    dmo: usize,
    penguin: Option<usize>,
    /// lying on the map (not carried, not swept away by a new drop)
    on_ground: bool,
}

#[derive(Default)]
struct PowerupPlugin {
    /// how many are dropped at a time (as many as the map has)
    count: usize,
    powerups: Vec<Powerup>,
    held: HashMap<usize, usize>,
    unheld: HashMap<(i32, i32), usize>,
    next_drop: Option<u32>,
    /// defense carriers: when each last warmed up by itself
    last_warm: HashMap<usize, u32>,
}

const VERSIONS: [(&str, f64); 3] = [("powerup", 1.3), ("gather_victory", 1.0), ("conquer_victory", 1.0)];

/// Start the plugins the map asks for (server / single player only).
pub fn init(w: &mut World) -> Result<Plugins, String> {
    let mut p = Plugins::default();
    let list = w.config.get("map", "required_plugins").unwrap_or("").to_string();
    for item in list.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (name, ver) = match item.split_once("-v") {
            Some((n, v)) => (n.trim(), v.trim().parse::<f64>().unwrap_or(0.0)),
            None => (item, 0.0),
        };
        let have = VERSIONS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).ok_or_else(|| format!("Could not find plugin {}", name))?;
        if ver > have {
            return Err(format!("{} plugin is too old. Required version is {}", name, ver));
        }
        match name {
            "conquer_victory" => {
                let tw = w.config.get("victory", "to_win").unwrap_or("all").trim().to_string();
                p.conquer = Some(Conquer {
                    to_win: if tw.chars().all(|c| c.is_ascii_digit()) && !tw.is_empty() { tw.parse().ok() } else { None },
                    to_lose: w.config.get_int("victory", "to_lose").unwrap_or(0) as i32,
                });
            }
            "gather_victory" => {
                let kind = w.config.get("victory", "resource_type").and_then(ResKind::from_name).ok_or("gather_victory: bad resource_type")?;
                let amount = w.config.get_int("victory", "amount").ok_or("gather_victory: no amount")? as i32;
                p.gather = Some(Gather { kind, amount });
            }
            "powerup" => {
                // the map says how many; where they land and which they are
                // is random (and changes with every drop)
                let mut pp = PowerupPlugin::default();
                for (_, params) in w.config.items("powerups") {
                    let name = params.split(',').next().unwrap_or("").trim();
                    PowerKind::from_name(name).ok_or_else(|| format!("Invalid powerup '{}'", name))?;
                    pp.count += 1;
                }
                drop_new(w, &mut pp);
                p.powerup = Some(pp);
            }
            _ => {}
        }
    }
    Ok(p)
}

/// The power-up this penguin carries (server / single player).
pub fn power_of(w: &World, ui: usize) -> Option<PowerKind> {
    let pp = w.plugins.powerup.as_ref()?;
    pp.held.get(&ui).map(|&pi| pp.powerups[pi].kind)
}

/// Where power-ups lie on the map (to be picked up).
pub fn powerups_on_ground(w: &World) -> Vec<(i32, i32)> {
    w.plugins.powerup.as_ref().map(|pp| pp.unheld.keys().copied().collect()).unwrap_or_default()
}

/// How far this penguin throws, in tiles (the same in every direction).
pub fn throw_range(w: &World, ui: usize) -> f64 {
    if power_of(w, ui) == Some(PowerKind::Range) {
        THROW_RANGE_POWER
    } else {
        THROW_RANGE
    }
}

/// How many snowballs this penguin makes and keeps when it isn't busy.
pub fn snowballs_kept(w: &World, ui: usize) -> i32 {
    if power_of(w, ui) == Some(PowerKind::DoubleSnowball) {
        2
    } else {
        1
    }
}

pub fn fire(w: &mut World, ev: Event) {
    match ev {
        Event::PenguinMove(u, x, y) => {
            if let Some(mut pp) = w.plugins.powerup.take() {
                if let Some(&pi) = pp.unheld.get(&(x, y)) {
                    if !pp.held.contains_key(&u) && w.units[u].kind == UnitKind::Snowballer && w.units[u].warmth > 0 {
                        pick_up(w, &mut pp, pi, u);
                    }
                }
                w.plugins.powerup = Some(pp);
            }
        }
        Event::PenguinReload(_) | Event::PenguinDirectHit(_, _) => {}
        Event::PenguinFrozen(u, by) => {
            if let Some(mut pp) = w.plugins.powerup.take() {
                if let Some(&pi) = pp.held.get(&u) {
                    // the thrower takes it, or it falls in the snow
                    let thief = by.filter(|&b| w.units[b].warmth > 0 && !pp.held.contains_key(&b) && w.units[b].kind == UnitKind::Snowballer);
                    detach(w, &mut pp, pi);
                    match thief {
                        Some(b) => attach(w, &mut pp, pi, b),
                        None => {
                            let at = (w.units[u].x, w.units[u].y);
                            let spot = free_spot_near(w, &pp, at);
                            put_down(w, &mut pp, pi, spot);
                        }
                    }
                }
                w.plugins.powerup = Some(pp);
            }
        }
        Event::PenguinGathered(_) => {}
        Event::PenguinUnloaded(_, kind) => {
            if w.plugins.gather.as_ref().is_some_and(|g| g.kind == kind) {
                check_victory(w);
            }
        }
        Event::RegionTakeover(_, _) => check_victory(w),
    }
}

/// Has a player, or a team of allies, won?  Conquer: every igloo belongs
/// to one team.  Gather: a team has the amount times its size (server /
/// single player).
pub fn check_victory(w: &mut World) {
    let ids: Vec<Pid> = w.players.keys().copied().collect();
    if let Some(g) = &w.plugins.gather {
        let (gk, amount) = (g.kind, g.amount);
        let mut done: Vec<Pid> = Vec::new();
        for &p in &ids {
            if done.contains(&p) {
                continue;
            }
            let team = crate::alliance::team_of(w, p);
            done.extend(team.iter().copied());
            let total: i32 = (0..w.buildings.len()).filter(|&b| w.buildings[b].player.is_some_and(|o| team.contains(&o))).map(|b| w.num_storage(b, gk)).sum();
            if total >= amount * team.len() as i32 {
                for &o in &ids {
                    w.set_victory(o, Some(team.contains(&o)));
                }
                return;
            }
        }
    }
    if w.plugins.conquer.is_some() {
        let mut counts: HashMap<Option<Pid>, i32> = HashMap::new();
        let mut owners: Vec<Option<Pid>> = Vec::new();
        let mut num = 0;
        for r in &w.regions {
            if r.igloo.is_some() {
                num += 1;
                *counts.entry(r.player).or_insert(0) += 1;
                owners.push(r.player);
            }
        }
        if let Some(c) = &mut w.plugins.conquer {
            c.to_win.get_or_insert(num);
        }
        // out of the game: no igloo left
        for &p in &ids {
            if counts.get(&Some(p)).copied().unwrap_or(0) == 0 {
                w.set_victory(p, Some(false));
            }
        }
        // every igloo held by one team
        if let Some(Some(first)) = owners.first().copied() {
            if owners.iter().all(|o| crate::alliance::friendly(w, *o, Some(first)) && o.is_some()) {
                let team = crate::alliance::team_of(w, first);
                for &o in &ids {
                    w.set_victory(o, Some(team.contains(&o)));
                }
            }
        }
    }
}

/// Every minute: what still lies on the map melts away and as many new
/// power-ups as the map has are dropped (server / single player).
pub fn tick(w: &mut World) {
    let Some(mut pp) = w.plugins.powerup.take() else { return };
    let now = w.now();
    // defense: the carrier slowly warms up by itself (not frozen, not in
    // an igloo, which warms it anyway)
    let carriers: Vec<usize> = pp.held.iter().filter(|(_, &pi)| pp.powerups[pi].kind == PowerKind::Defense).map(|(&u, _)| u).collect();
    pp.last_warm.retain(|u, _| carriers.contains(u));
    for u in carriers {
        let un = &w.units[u];
        let warming = un.warmth > 0 && un.warmth < un.max_warmth && !matches!(un.state, State::InBuilding | State::Entering | State::Frozen);
        let last = *pp.last_warm.entry(u).or_insert(now);
        if !warming {
            pp.last_warm.insert(u, now);
        } else if now.wrapping_sub(last) >= DEFENSE_WARM_EVERY_MS {
            let v = w.units[u].warmth + 1;
            w.set_warmth(u, v);
            pp.last_warm.insert(u, now);
        }
    }
    let count = pp.count;
    match pp.next_drop {
        _ if count == 0 => {}
        None => {
            pp.next_drop = Some(now.wrapping_add(DROP_EVERY_MS));
            announce(w, DROP_EVERY_MS, false, None);
        }
        Some(t) if (now.wrapping_sub(t) as i32) >= 0 => {
            drop_new(w, &mut pp);
            pp.next_drop = Some(now.wrapping_add(DROP_EVERY_MS));
            announce(w, DROP_EVERY_MS, true, None);
        }
        _ => {}
    }
    w.plugins.powerup = Some(pp);
}

/// The players' countdowns go by this "chat message" from nobody: older
/// versions (and the Python one) leave messages from unknown players out.
pub const TIMER_FROM: u16 = 0xfffe;
const TIMER_PREFIX: &str = "#powerup-timer ";

/// Start the countdown to the next drop here and (hosting) for everybody,
/// or only for the player on connection `to`.
pub fn announce(w: &mut World, ms_left: u32, dropped: bool, to: Option<crate::net::ConnId>) {
    if to.is_none() {
        timer_heard(w, ms_left, dropped);
    }
    if w.role == Role::Server {
        let m = crate::net::Msg::ChatMessage { msg: format!("{}{} {}", TIMER_PREFIX, ms_left, dropped as u8), from_id: TIMER_FROM, to_id: 0xffff };
        match to {
            Some(c) => w.send_to(m, &[c], &[]),
            None => w.send(m),
        }
    }
}

/// Hidden "chat messages" (from nobody, see `TIMER_FROM`) that carry what
/// the original protocol has no message for.
const KIND_PREFIX: &str = "#unit-kind ";
const SWITCH_PREFIX: &str = "#switch-class ";

/// Host: tell everybody a penguin changed class.
pub fn send_unit_kind(w: &mut World, ui: usize) {
    if w.role == Role::Server {
        let m = crate::net::Msg::ChatMessage { msg: format!("{}{} {}", KIND_PREFIX, ui, w.units[ui].kind.name()), from_id: TIMER_FROM, to_id: 0xffff };
        w.send(m);
    }
}

/// Client: ask the host to send these penguins home to change class.
pub fn request_switch(w: &mut World, ids: &[usize]) {
    let list: Vec<String> = ids.iter().map(|u| u.to_string()).collect();
    let m = crate::net::Msg::ChatMessage { msg: format!("{}{}", SWITCH_PREFIX, list.join(",")), from_id: TIMER_FROM, to_id: 0xffff };
    w.send(m);
}

/// Host: a hidden message from the player `from` (true if it was one).
pub fn hidden_request(w: &mut World, msg: &str, from_id: u16, from: Option<Pid>) -> bool {
    if from_id != TIMER_FROM {
        return false;
    }
    if crate::alliance::host_message(w, msg, from) || crate::wall::host_message(w, msg, from) {
        return true;
    }
    if let Some(rest) = msg.strip_prefix(SWITCH_PREFIX) {
        let ids: Vec<usize> = rest.split(',').filter_map(|v| v.trim().parse().ok()).filter(|&u| u < w.units.len()).collect();
        if from.is_some() {
            crate::unit::order_switch(w, from, &ids);
        }
    }
    true
}

/// A client heard a hidden message from the host (true if it was one).
pub fn timer_message(w: &mut World, msg: &str, from_id: u16) -> bool {
    if from_id != TIMER_FROM {
        return false;
    }
    if crate::alliance::client_message(w, msg) || crate::wall::client_message(w, msg) || crate::stats::client_message(w, msg) || crate::ai::client_message(w, msg) {
        return true;
    }
    if let Some(rest) = msg.strip_prefix(KIND_PREFIX) {
        let mut it = rest.split(' ');
        let u = it.next().and_then(|v| v.parse::<usize>().ok());
        let kind = match it.next() {
            Some("worker") => Some(UnitKind::Worker),
            Some("snowballer") => Some(UnitKind::Snowballer),
            _ => None,
        };
        if let (Some(u), Some(k)) = (u, kind) {
            if u < w.units.len() {
                w.set_unit_kind(u, k);
                w.units[u].switch_pending = false;
                w.units[u].switch_done_at = None;
            }
        }
        return true;
    }
    if let Some(rest) = msg.strip_prefix(TIMER_PREFIX) {
        let mut it = rest.split(' ');
        if let Some(ms) = it.next().and_then(|v| v.parse::<u32>().ok()) {
            let dropped = it.next() == Some("1");
            timer_heard(w, ms, dropped);
        }
    }
    true
}

fn timer_heard(w: &mut World, ms_left: u32, dropped: bool) {
    let now = w.now();
    w.powerup_drop_at = Some(now.wrapping_add(ms_left));
    if dropped {
        w.powerup_dropped_at = Some(now);
    }
}

/// Milliseconds until the next drop (None: no drops on this map).
pub fn next_drop_in(w: &World) -> Option<u32> {
    let pp = w.plugins.powerup.as_ref()?;
    let t = pp.next_drop?;
    Some(t.wrapping_sub(w.now()).min(DROP_EVERY_MS))
}

fn drop_new(w: &mut World, pp: &mut PowerupPlugin) {
    for pi in 0..pp.powerups.len() {
        if pp.powerups[pi].on_ground {
            let (dmo, pos) = (pp.powerups[pi].dmo, pp.powerups[pi].position);
            w.set_dmo_hidden(dmo, true);
            pp.unheld.remove(&pos);
            pp.powerups[pi].on_ground = false;
        }
    }
    let mut taken: Vec<(i32, i32)> = Vec::new();
    for _ in 0..pp.count {
        let kind = ALL_POWERS[w.rng.randint(0, ALL_POWERS.len() as i32 - 1) as usize];
        let Some(spot) = random_spot(w, pp, &taken) else { continue };
        taken.push(spot);
        // reuse a power-up nobody has any more, or make one
        let pi = match (0..pp.powerups.len()).find(|&i| !pp.powerups[i].on_ground && pp.powerups[i].penguin.is_none()) {
            Some(i) => i,
            None => {
                pp.powerups.push(Powerup { kind, position: spot, dmo: usize::MAX, penguin: None, on_ground: false });
                pp.powerups.len() - 1
            }
        };
        pp.powerups[pi].kind = kind;
        put_down(w, pp, pi, spot);
    }
}

/// Can a power-up lie here?
fn spot_ok(w: &World, pp: &PowerupPlugin, pos: (i32, i32)) -> bool {
    match w.tile(pos.0, pos.1) {
        Some(t) => t.resource.is_none() && t.building.is_none() && !pp.unheld.contains_key(&pos),
        None => false,
    }
}

/// A random place on the map, away from igloos and from the other new
/// power-ups.
fn random_spot(w: &mut World, pp: &PowerupPlugin, taken: &[(i32, i32)]) -> Option<(i32, i32)> {
    let far = |a: (i32, i32), b: (i32, i32), d: i32| (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2) >= d * d;
    for attempt in 0..600 {
        // (less choosy when the map is crowded)
        let (from_igloo, apart) = if attempt < 300 { (6, 10) } else if attempt < 500 { (4, 5) } else { (2, 1) };
        let pos = (w.rng.randint(0, w.size.0 - 1), w.rng.randint(0, w.size.1 - 1));
        if !spot_ok(w, pp, pos) {
            continue;
        }
        let near_igloo = w.buildings.iter().any(|b| {
            let c = (b.x + IGLOO_SIZE / 2, b.y + IGLOO_SIZE / 2);
            !far(pos, c, from_igloo + IGLOO_SIZE / 2)
        });
        if near_igloo || taken.iter().any(|&t| !far(pos, t, apart)) {
            continue;
        }
        return Some(pos);
    }
    None
}

/// The nearest free tile to `at` (where a frozen carrier drops it).
fn free_spot_near(w: &World, pp: &PowerupPlugin, at: (i32, i32)) -> (i32, i32) {
    for r in 0..6 {
        for dy in -r..=r {
            for dx in -r..=r {
                let p = (at.0 + dx, at.1 + dy);
                if (dx.abs() == r || dy.abs() == r) && spot_ok(w, pp, p) {
                    return p;
                }
            }
        }
    }
    at
}

/// Lay a power-up on the map (with a new picture, which reaches the
/// players in the game too).
fn put_down(w: &mut World, pp: &mut PowerupPlugin, pi: usize, pos: (i32, i32)) {
    let kind = pp.powerups[pi].kind;
    let mini = "plugins/powerup/minipowerup.png".to_string();
    w.dmos.push(Dmo::new(kind.image(), pos, false, false, Some(mini.clone()), true));
    let dmo = w.dmos.len() - 1;
    if w.role == Role::Server {
        w.send(crate::net::Msg::CreateDmo {
            dmo_id: dmo as u16,
            image_name: kind.image().to_string(),
            position: (pos.0 as u16, pos.1 as u16),
            obstruction: false,
            hidden: false,
            minimapimage: mini,
        });
    }
    let p = &mut pp.powerups[pi];
    p.dmo = dmo;
    p.position = pos;
    p.on_ground = true;
    p.penguin = None;
    pp.unheld.insert(pos, pi);
}

fn pick_up(w: &mut World, pp: &mut PowerupPlugin, pi: usize, penguin: usize) {
    let by = w.igloo_player(penguin);
    crate::stats::add(w, by, |s| s.powerups += 1);
    let (dmo, pos) = (pp.powerups[pi].dmo, pp.powerups[pi].position);
    w.set_dmo_hidden(dmo, true);
    pp.unheld.remove(&pos);
    pp.powerups[pi].on_ground = false;
    attach(w, pp, pi, penguin);
}

/// A penguin starts carrying it.
fn attach(w: &mut World, pp: &mut PowerupPlugin, pi: usize, penguin: usize) {
    let kind = pp.powerups[pi].kind;
    pp.powerups[pi].penguin = Some(penguin);
    pp.held.insert(penguin, pi);
    w.add_emblem(penguin, kind.emblem());
    if kind == PowerKind::Defense {
        w.units[penguin].max_warmth += DEFENSE_WARMTH;
        let v = w.units[penguin].warmth + DEFENSE_WARMTH;
        w.set_warmth(penguin, v);
    }
}

/// Its penguin loses it.
fn detach(w: &mut World, pp: &mut PowerupPlugin, pi: usize) {
    let Some(penguin) = pp.powerups[pi].penguin.take() else { return };
    let kind = pp.powerups[pi].kind;
    pp.held.remove(&penguin);
    w.remove_emblem(penguin, kind.image());
    if kind == PowerKind::Defense {
        let u = &mut w.units[penguin];
        u.max_warmth = (u.max_warmth - DEFENSE_WARMTH).max(1);
        if u.warmth > u.max_warmth {
            let m = u.max_warmth;
            w.set_warmth(penguin, m);
        }
    }
}

/// A penguin stops being a snowballer: its power-up drops by the igloo.
pub fn drop_power(w: &mut World, ui: usize) {
    let Some(mut pp) = w.plugins.powerup.take() else { return };
    if let Some(&pi) = pp.held.get(&ui) {
        detach(w, &mut pp, pi);
        let ig = w.units[ui].igloo;
        let door = (w.buildings[ig].x - 1, w.buildings[ig].y - 1);
        let spot = free_spot_near(w, &pp, door);
        put_down(w, &mut pp, pi, spot);
    }
    w.plugins.powerup = Some(pp);
}

/// For the test stats line: power-ups on the map and carried, by kind.
pub fn summary(w: &World) -> String {
    let Some(pp) = w.plugins.powerup.as_ref() else { return String::new() };
    let mut ground: Vec<String> = pp.powerups.iter().filter(|p| p.on_ground).map(|p| format!("{:?}@{},{}", p.kind, p.position.0, p.position.1)).collect();
    ground.sort();
    let held: Vec<String> = pp.held.iter().map(|(&u, &pi)| format!("{}:{:?}:w{}/{}:{:?}", u, pp.powerups[pi].kind, w.units[u].warmth, w.units[u].max_warmth, w.units[u].state)).collect();
    format!("count={} ground=[{}] held=[{}]", pp.count, ground.join(" "), held.join(" "))
}

/// Test hook: power-ups of this kind for `player`'s snowballers
/// (SNOWBALLZ_GIVE_POWER): as many as the map drops at a time.
pub fn test_give(w: &mut World, kind_name: &str, player: Option<Pid>) {
    let Some(kind) = PowerKind::from_name(kind_name) else { return };
    let Some(mut pp) = w.plugins.powerup.take() else { return };
    let list: Vec<usize> = (0..w.units.len()).filter(|&u| w.igloo_player(u) == player && w.units[u].kind == UnitKind::Snowballer && !pp.held.contains_key(&u)).collect();
    for u in list.into_iter().take(pp.count) {
        pp.powerups.push(Powerup { kind, position: (0, 0), dmo: usize::MAX, penguin: None, on_ground: false });
        let pi = pp.powerups.len() - 1;
        attach(w, &mut pp, pi, u);
    }
    w.plugins.powerup = Some(pp);
}

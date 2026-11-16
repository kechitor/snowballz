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

#[derive(Clone, Copy, PartialEq, Debug)]
enum PowerKind {
    DoubleSnowball,
    Defense,
}

impl PowerKind {
    fn image(&self) -> &'static str {
        match self {
            PowerKind::DoubleSnowball => "plugins/powerup/powerup_1.png",
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
    dmo: usize,
    penguin: Option<usize>,
}

#[derive(Default)]
struct PowerupPlugin {
    powerups: Vec<Powerup>,
    held: HashMap<usize, usize>,
    unheld: HashMap<(i32, i32), usize>,
}

const VERSIONS: [(&str, f64); 3] = [("powerup", 1.2), ("gather_victory", 1.0), ("conquer_victory", 1.0)];

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
                let mut pp = PowerupPlugin::default();
                for (_, params) in w.config.items("powerups") {
                    let parts: Vec<&str> = params.split(',').map(|s| s.trim()).collect();
                    let kind = match parts.first().copied() {
                        Some("DoubleSnowball") => PowerKind::DoubleSnowball,
                        Some("Defense") => PowerKind::Defense,
                        other => return Err(format!("Invalid powerup '{}'", other.unwrap_or(""))),
                    };
                    let x: i32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let y: i32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    w.dmos.push(Dmo::new(kind.image(), (x, y), false, false, Some("plugins/powerup/minipowerup.png".into()), true));
                    let dmo = w.dmos.len() - 1;
                    pp.unheld.insert((x, y), pp.powerups.len());
                    pp.powerups.push(Powerup { kind, position: (x, y), dmo, penguin: None });
                }
                p.powerup = Some(pp);
            }
            _ => {}
        }
    }
    Ok(p)
}

pub fn fire(w: &mut World, ev: Event) {
    match ev {
        Event::PenguinMove(u, x, y) => {
            if let Some(mut pp) = w.plugins.powerup.take() {
                if let Some(&pi) = pp.unheld.get(&(x, y)) {
                    if !pp.held.contains_key(&u) && w.units[u].kind == UnitKind::Snowballer {
                        give_to(w, &mut pp, pi, u);
                    }
                }
                w.plugins.powerup = Some(pp);
            }
        }
        Event::PenguinReload(u) => {
            if let Some(pp) = &w.plugins.powerup {
                if let Some(&pi) = pp.held.get(&u) {
                    if pp.powerups[pi].kind == PowerKind::DoubleSnowball {
                        w.units[u].snowballs = 2;
                    }
                }
            }
        }
        Event::PenguinDirectHit(u, d) => {
            let defended = w
                .plugins
                .powerup
                .as_ref()
                .and_then(|pp| pp.held.get(&u).map(|&pi| pp.powerups[pi].kind == PowerKind::Defense))
                .unwrap_or(false);
            if defended {
                let v = w.units[u].warmth + d - 1;
                w.set_warmth(u, v);
            }
        }
        Event::PenguinFrozen(u, by) => {
            if let Some(mut pp) = w.plugins.powerup.take() {
                if let Some(&pi) = pp.held.get(&u) {
                    match by {
                        Some(b) if w.units[b].warmth > 0 && !pp.held.contains_key(&b) => give_to(w, &mut pp, pi, b),
                        _ => take_from(w, &mut pp, pi),
                    }
                }
                w.plugins.powerup = Some(pp);
            }
        }
        Event::PenguinGathered(_) => {}
        Event::PenguinUnloaded(u, kind) => {
            let Some(g) = &w.plugins.gather else { return };
            if kind != g.kind {
                return;
            }
            let (gk, amount) = (g.kind, g.amount);
            let owner = w.units[u].player;
            let total: i32 = (0..w.buildings.len()).filter(|&b| w.buildings[b].player == owner).map(|b| w.num_storage(b, gk)).sum();
            if total >= amount {
                let ids: Vec<Pid> = w.players.keys().copied().collect();
                for p in ids {
                    w.set_victory(p, Some(Some(p) == owner));
                }
            }
        }
        Event::RegionTakeover(_, _) => {
            let Some(c) = &mut w.plugins.conquer else { return };
            let mut counts: HashMap<Option<Pid>, i32> = HashMap::new();
            let mut num = 0;
            for r in &w.regions {
                if r.igloo.is_some() {
                    num += 1;
                    *counts.entry(r.player).or_insert(0) += 1;
                }
            }
            let to_win = *c.to_win.get_or_insert(num);
            let ids: Vec<Pid> = w.players.keys().copied().collect();
            for p in ids.iter().copied() {
                match counts.get(&Some(p)) {
                    Some(&n) if n == 0 => w.set_victory(p, Some(false)),
                    Some(&n) if n >= to_win => {
                        w.set_victory(p, Some(true));
                        for &o in &ids {
                            if o != p {
                                w.set_victory(o, Some(false));
                            }
                        }
                    }
                    Some(_) => {}
                    None => w.set_victory(p, Some(false)),
                }
            }
        }
    }
}

fn give_to(w: &mut World, pp: &mut PowerupPlugin, pi: usize, penguin: usize) {
    if pp.powerups[pi].penguin.is_some() {
        take_from(w, pp, pi);
    }
    let p = &mut pp.powerups[pi];
    p.penguin = Some(penguin);
    let (kind, dmo, pos) = (p.kind, p.dmo, p.position);
    w.set_dmo_hidden(dmo, true);
    w.add_emblem(penguin, kind.emblem());
    pp.unheld.remove(&pos);
    pp.held.insert(penguin, pi);
    if kind == PowerKind::Defense {
        w.units[penguin].max_warmth += 4;
        let v = w.units[penguin].warmth + 4;
        w.set_warmth(penguin, v);
    }
}

fn take_from(w: &mut World, pp: &mut PowerupPlugin, pi: usize) {
    let Some(penguin) = pp.powerups[pi].penguin else { return };
    let (kind, dmo, pos) = (pp.powerups[pi].kind, pp.powerups[pi].dmo, pp.powerups[pi].position);
    if kind == PowerKind::Defense {
        w.units[penguin].max_warmth -= 4;
    }
    w.remove_emblem(penguin, kind.image());
    pp.held.remove(&penguin);
    pp.unheld.insert(pos, pi);
    w.set_dmo_hidden(dmo, false);
    pp.powerups[pi].penguin = None;
}

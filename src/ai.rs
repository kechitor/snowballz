//! The computer players (ai.py): an offensive and a revengeful
//! personality, guarding, gathering, and some trash talk.

use crate::alliance;
use crate::unit;
use crate::world::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub struct Ai {
    pub pid: Pid,
    units: BTreeSet<usize>,
    attacking: BTreeSet<usize>,
    guarding: BTreeSet<usize>,
    being_attacked: bool,
    warring: BTreeSet<Pid>,
    spite: HashMap<String, i32>,
    regions: BTreeSet<usize>,
    pub send_msgs: Vec<String>,
    workers: Vec<usize>,
    // the revengeful personality
    grudges: BTreeSet<Pid>,
    last_spite_increase: u32,
    last_grudge_release: u32,
    // alliances
    next_offer: Option<u32>,
}

impl Ai {
    pub fn new(pid: Pid) -> Ai {
        Ai { pid, ..Default::default() }
    }
}

fn take(w: &mut World, pid: Pid) -> Option<Ai> {
    w.players.get_mut(&pid).map(|p| std::mem::take(&mut p.ai))
}

fn put(w: &mut World, pid: Pid, ai: Ai) {
    if let Some(p) = w.players.get_mut(&pid) {
        p.ai = ai;
    }
}

/// What the AI answers a human's chat (and it may hold a grudge).
pub fn reply(w: &mut World, pid: Pid, msg: &str, from: &str) -> String {
    let msg = msg.to_lowercase();
    let words: Vec<&str> = msg.split_whitespace().collect();
    let has = |x: &str| words.contains(&x);
    let mut to_player = true;
    let r = if has("hi") || has("hello") {
        "Hi".to_string()
    } else if msg.contains("bye") {
        format!("Yes, {}, adieu!", from)
    } else if has("how") && has("are") && has("you") {
        "Better than a n00b like you!".into()
    } else if (msg.contains("freez") || msg.contains("froz") || msg.contains("cold")) && msg.contains("you") {
        "Don't worry, I'll be sure to make it up to you!".into()
    } else if msg.contains("die") {
        if let Some(p) = w.players.get_mut(&pid) {
            p.ai.spite.insert(from.to_string(), 5);
        }
        "No no, YOU die! MUHAHAHAHA".into()
    } else if msg.contains("dum") || msg.contains("stupid") || msg.contains("idiot") {
        "I'll take that as a compliment".into()
    } else if msg.contains("fair") {
        "Life isn't fair!".into()
    } else if msg.contains("bot") {
        "What's a bot? I'm not a bot! You're the bot! DIE BOT!".into()
    } else if has("go") && has("away") {
        "I'll go away when I want to!".into()
    } else if msg.contains("no") {
        "YEEEEES!".into()
    } else if msg.contains("attack") || msg.contains("stop") {
        "Uuu... no!".into()
    } else if msg.contains("thank") {
        "You're quite welcome!".into()
    } else if msg.contains("loser") {
        to_player = false;
        format!("All hail {}, king of the losers!", from)
    } else {
        "Stop babbling".into()
    };
    if to_player {
        format!(">{} {}", from, r)
    } else {
        r
    }
}

/// Recount regions and penguins (whenever a region changes hands).
pub fn setup(w: &mut World, pid: Pid) {
    let Some(mut ai) = take(w, pid) else { return };
    for (ri, r) in w.regions.iter().enumerate() {
        if r.player == Some(pid) {
            ai.regions.insert(ri);
        }
    }
    ai.units.clear();
    ai.workers.clear();
    for ui in 0..w.units.len() {
        if w.igloo_player(ui) == Some(pid) {
            if w.units[ui].kind == UnitKind::Snowballer {
                ai.units.insert(ui);
            } else {
                ai.workers.push(ui);
            }
        }
    }
    // gather what the map asks for
    if let Some(g) = w.ai_cfg.gather {
        for &wk in &ai.workers {
            unit::disban(w, wk);
        }
        let n = (ai.workers.len() as f64 * (w.ai_cfg.gather_force as f64 / 100.0)) as usize;
        for &wk in ai.workers.iter().take(n) {
            w.units[wk].going_for = Some(g);
        }
    }
    put(w, pid, ai);
}

fn spite(ai: &mut Ai, name: &str) -> i32 {
    *ai.spite.entry(name.to_string()).or_insert(0)
}

fn offensive_attack_region(w: &World, ai: &Ai) -> i32 {
    if (ai.units.len() as i32) - (ai.attacking.len() as i32) < 4 {
        return -10;
    }
    let warm = ai.units.iter().filter(|&&u| w.units[u].warmth >= 5).count();
    if warm < 5 {
        return -20;
    }
    if ai.being_attacked {
        return -5;
    }
    if !ai.warring.is_empty() {
        return -8;
    }
    5
}

fn revengeful_attack_region(w: &World, ai: &mut Ai, ri: usize) -> i32 {
    match w.regions[ri].player {
        None => 0,
        Some(p) => {
            let name = w.player_name(p);
            spite(ai, &name).min(7)
        }
    }
}

fn attacked_by(w: &mut World, ai: &mut Ai, enemy: Option<Pid>) {
    let Some(enemy) = enemy else { return };
    let name = w.player_name(enemy);
    spite(ai, &name);
    let now = w.now();
    if ai.last_spite_increase.wrapping_add(20000) < now {
        *ai.spite.get_mut(&name).unwrap() += 1;
        ai.last_spite_increase = now;
    }
    ai.warring.clear();
    if ai.last_grudge_release.wrapping_add(30000) < now {
        ai.grudges.clear();
        ai.last_grudge_release = now;
    }
    if ai.grudges.insert(enemy) {
        let s = ai.spite[&name];
        let text = if s > 6 {
            None
        } else if s > 4 {
            Some("Dirty swine, leave me alone!")
        } else if s > 2 {
            Some("*SIGH*, just leave me alone will you?!")
        } else if s > 1 {
            Some("I see you! Just what do you think you're doing in MY territory?")
        } else if s > 0 {
            Some("*AHEM* I would recommend that you leave me alone!")
        } else {
            None
        };
        if let Some(t) = text {
            w.player_send_msg(ai.pid, &format!(">{} {}", name, t));
        }
    }
}

fn attack(w: &mut World, ai: &mut Ai, ri: usize) {
    let Some(enemy) = w.regions[ri].player else { return };
    let target = w.buildings.iter().find(|b| b.region == ri).map(|b| (b.x, b.y));
    let max = ai.units.difference(&ai.attacking).count();
    let mut selected = Vec::new();
    for &u in &ai.units {
        if selected.len() >= max {
            break;
        }
        if !ai.attacking.contains(&u) && w.units[u].warmth > 4 {
            selected.push(u);
        }
    }
    for u in selected {
        w.units[u].target_pos = target;
        ai.attacking.insert(u);
    }
    if ai.warring.insert(enemy) {
        let name = w.player_name(enemy);
        let s = spite(ai, &name);
        let text = if s > 6 {
            None
        } else if s > 4 {
            Some("DIE DIE DIE!!!")
        } else if s > 3 {
            Some("You are getting on my nerves... prepare to die!")
        } else if s > 1 {
            Some("hmm... I think I'll attack... YOU! MUHAHAHA!!")
        } else {
            Some("I think I'll try picking on you... no hard feelings")
        };
        if let Some(t) = text {
            w.player_send_msg(ai.pid, &format!(">{} {}", name, t));
        }
    }
}

/// Could this player and `other`'s team join without leaving nobody to
/// fight (an AI doesn't make an alliance that ends the game at once)?
fn team_ok(w: &World, pid: Pid, other: Pid) -> bool {
    let size = alliance::team_of(w, pid).len() + alliance::team_of(w, other).len();
    let alive = w.players.keys().filter(|&&p| alliance::alive(w, p)).count();
    size < alive && size <= (alive / 2).max(2)
}

/// Answer offers and now and then make one.
fn diplomacy(w: &mut World, ai: &mut Ai) {
    let pid = ai.pid;
    let now = w.now();
    // offers to me (give it a moment's thought)
    let mine: Vec<(Pid, u32)> = w.offers.iter().filter(|o| o.1 == pid).map(|o| (o.0, o.2)).collect();
    for (from, when) in mine {
        if now.wrapping_sub(when) < 4000 {
            continue;
        }
        let name = w.player_name(from);
        let grudge = spite(ai, &name) >= 3 || ai.grudges.contains(&from);
        let rich = alliance::fish_of(w, pid) >= alliance::ALLIANCE_FISH;
        let yes = rich && !grudge && team_ok(w, pid, from) && w.rng.random() < 0.65;
        let text = if yes {
            "Deal! Together we'll bury them in snow."
        } else if !rich {
            "I'd love to, but I can't spare the fish."
        } else if grudge {
            "After what you did? Not a chance!"
        } else {
            "Thanks, but I work alone."
        };
        w.player_send_msg(pid, &format!(">{} {}", name, text));
        alliance::answer(w, from, pid, yes);
        if yes {
            // no more attacks on the new ally
            ai.warring.remove(&from);
            ai.grudges.remove(&from);
        }
    }
    // an offer of my own, every few minutes
    let next = *ai.next_offer.get_or_insert(now.wrapping_add(w.rng.randint(60_000, 150_000) as u32));
    if (now.wrapping_sub(next) as i32) < 0 {
        return;
    }
    ai.next_offer = Some(now.wrapping_add(w.rng.randint(90_000, 200_000) as u32));
    if alliance::fish_of(w, pid) < alliance::ALLIANCE_FISH + 10 || w.rng.random() < 0.4 {
        return;
    }
    let candidates: Vec<Pid> = w
        .players
        .keys()
        .copied()
        .filter(|&p| p != pid && alliance::alive(w, p) && !alliance::allied(w, Some(pid), Some(p)))
        .filter(|&p| !ai.warring.contains(&p) && !ai.grudges.contains(&p) && team_ok(w, pid, p))
        .filter(|&p| !w.offers.iter().any(|o| (o.0 == pid && o.1 == p) || (o.0 == p && o.1 == pid)))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let to = candidates[w.rng.randint(0, candidates.len() as i32 - 1) as usize];
    let name = w.player_name(to);
    if spite(ai, &name) >= 3 {
        return;
    }
    w.player_send_msg(pid, &format!(">{} How about an alliance? We could take on the others together.", name));
    alliance::offer(w, pid, to);
}

/// One AI turn (every 3 seconds).
pub fn run(w: &mut World, pid: Pid) {
    let Some(mut ai) = take(w, pid) else { return };
    for m in std::mem::take(&mut ai.send_msgs) {
        w.player_send_msg(pid, &m);
    }
    diplomacy(w, &mut ai);
    // allies are left alone
    let allies: Vec<Pid> = alliance::team_of(w, pid).into_iter().filter(|&p| p != pid).collect();
    ai.warring.retain(|p| !allies.contains(p));
    ai.grudges.retain(|p| !allies.contains(p));
    let recall: Vec<usize> = ai
        .attacking
        .iter()
        .copied()
        .filter(|&u| {
            let t = w.units[u].target_pos.and_then(|(x, y)| w.tile(x, y)).map(|t| w.regions[t.region].player);
            t.is_some_and(|p| p.is_some_and(|p| allies.contains(&p)))
        })
        .collect();
    for u in recall {
        w.units[u].target_pos = None;
        ai.attacking.remove(&u);
    }

    // has it taken over a region?
    for ri in 0..w.regions.len() {
        if w.regions[ri].player == Some(pid) && !ai.regions.contains(&ri) {
            ai.regions.insert(ri);
            for &u in &ai.attacking {
                w.units[u].target_pos = None;
                w.units[u].set_go_to_building(None);
            }
            ai.attacking.clear();
            ai.warring.clear();
            break;
        }
    }

    // guard somewhere?
    if let Some(gp) = w.ai_cfg.guard_pos {
        if ai.guarding.is_empty() {
            let free: Vec<usize> = ai.units.difference(&ai.attacking).copied().collect();
            let limit = free.len() / 3;
            let mut n = 0;
            for u in free {
                if n > limit {
                    break;
                }
                ai.guarding.insert(u);
                n += 1;
            }
        }
        for &u in &ai.guarding {
            w.units[u].target_pos = Some(gp);
        }
        ai.units = ai.units.difference(&ai.guarding).copied().collect();
    }

    // attack somebody?
    let mut best: Option<(usize, i32)> = None;
    for ri in 0..w.regions.len() {
        if alliance::friendly(w, w.regions[ri].player, Some(pid)) {
            continue;
        }
        let amount = offensive_attack_region(w, &ai) + revengeful_attack_region(w, &mut ai, ri);
        match best {
            None => best = Some((ri, amount)),
            Some((_, b)) => {
                if amount > b || (amount == b && w.rng.randint(0, 1) == 1) {
                    best = Some((ri, amount));
                }
            }
        }
    }
    if let Some((ri, amount)) = best {
        if amount > 0 {
            attack(w, &mut ai, ri);
        }
    }
    let done: Vec<usize> = ai
        .attacking
        .iter()
        .copied()
        .filter(|&u| w.units[u].target_pos.is_none() && w.units[u].go_to_building().is_none())
        .collect();
    for u in done {
        ai.attacking.remove(&u);
    }
    if ai.attacking.len() < 3 {
        // retreat!
        for u in std::mem::take(&mut ai.attacking) {
            w.units[u].target_pos = None;
            let ig = w.units[u].igloo;
            w.units[u].set_go_to_building(Some(ig));
            ai.warring.clear();
        }
    }

    // enemies in our regions?
    let mut enemy_in: BTreeMap<usize, (i32, Option<Pid>)> = BTreeMap::new();
    for ui in 0..w.units.len() {
        let u = &w.units[ui];
        let Some(t) = w.tile(u.x, u.y) else { continue };
        if w.regions[t.region].player == Some(pid) && !alliance::friendly(w, w.igloo_player(ui), Some(pid)) && u.warmth > 4 {
            enemy_in.entry(t.region).and_modify(|e| e.0 += 1).or_insert((1, u.player));
        }
    }
    ai.being_attacked = false;
    for (ri, (n, enemy)) in enemy_in {
        if n >= 4 {
            // defend!
            ai.being_attacked = true;
            for &u in &ai.attacking {
                if w.buildings[w.units[u].igloo].region == ri {
                    w.units[u].target_pos = None;
                }
            }
            attacked_by(w, &mut ai, enemy);
        }
    }

    // can we take an igloo over?
    let mut attacking_in: BTreeMap<usize, (i32, usize)> = BTreeMap::new();
    for &u in &ai.attacking {
        if let Some(t) = w.tile(w.units[u].x, w.units[u].y) {
            attacking_in.entry(t.region).and_modify(|e| e.0 += 1).or_insert((1, u));
        }
    }
    for (ri, (n, u)) in attacking_in {
        let Some(ig) = w.regions[ri].igloo else { continue };
        if n >= 2 {
            let (ix, iy) = (w.buildings[ig].x, w.buildings[ig].y);
            let enemies = w
                .units_around(ix, iy)
                .into_iter()
                .filter(|&o| !alliance::friendly(w, w.igloo_player(o), Some(pid)) && w.units[o].kind == UnitKind::Snowballer && w.units[o].warmth > 4)
                .count();
            if enemies <= 1 {
                w.units[u].set_go_to_building(Some(ig));
                w.units[u].last_tiles.clear();
                w.units[u].target_pos = None;
            }
        }
    }
    put(w, pid, ai);
}

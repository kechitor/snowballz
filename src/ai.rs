//! The computer players (ai.py): an offensive and a revengeful
//! personality, guarding, gathering, and some trash talk; and on top of
//! that (smarts below): a difficulty of their own, a strategy that follows
//! the situation (build up, attack, defend), defending their igloos,
//! picking up power-ups, changing penguins' classes and building walls.

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
    // the smarts
    next_think: u32,
    pub mode: Mode,
    next_switch: u32,
    next_wall: u32,
    walls_planned: u32,
    /// when the strategy last changed
    mode_since: u32,
    /// snowballers sent to pick up a power-up: where it lies
    runners: BTreeMap<usize, (i32, i32)>,
    /// snowballers sent home to defend
    defenders: BTreeSet<usize>,
}

/// How well a computer player plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiLevel {
    /// slow to act, poor aim, no power-ups, class changes or walls
    Easy,
    #[default]
    Normal,
    /// quick, good aim, everything
    Hard,
}

impl AiLevel {
    pub const ALL: [AiLevel; 3] = [AiLevel::Easy, AiLevel::Normal, AiLevel::Hard];

    pub fn label(self) -> &'static str {
        match self {
            AiLevel::Easy => "Easy",
            AiLevel::Normal => "Normal",
            AiLevel::Hard => "Hard",
        }
    }

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> AiLevel {
        *AiLevel::ALL.get(i as usize).unwrap_or(&AiLevel::Normal)
    }

    pub fn next(self) -> AiLevel {
        AiLevel::from_index((self.index() + 1) % 3)
    }

    /// How often it thinks (ms).
    fn think_ms(self) -> u32 {
        match self {
            AiLevel::Easy => 5000,
            AiLevel::Normal => 3000,
            AiLevel::Hard => 1500,
        }
    }

    /// Its snowballers' aim: the share of further throws on target,
    /// against a player's.
    pub fn aim(self) -> f64 {
        match self {
            AiLevel::Easy => 0.6,
            AiLevel::Normal => 1.0,
            AiLevel::Hard => 1.3,
        }
    }

    /// It attacks when its warm snowballers outnumber the strongest
    /// enemy's this many times.
    fn attack_ratio(self) -> f64 {
        match self {
            AiLevel::Easy => 1.6,
            AiLevel::Normal => 1.2,
            AiLevel::Hard => 0.95,
        }
    }
}

/// The default level for new computer players (Settings).
pub fn default_level() -> AiLevel {
    AiLevel::from_index(crate::settings::get_option("ailevel").trim().parse().unwrap_or(1))
}

/// A player's level (Normal for people).
pub fn level_of(w: &World, p: Option<Pid>) -> AiLevel {
    p.and_then(|p| w.players.get(&p)).filter(|p| p.kind == PlayerKind::Ai).map(|p| p.ai_level).unwrap_or_default()
}

const LEVEL_PREFIX: &str = "#ai-level ";

/// Set a computer player's level (the host; everybody is told).
pub fn set_level(w: &mut World, pid: Pid, level: AiLevel) {
    let Some(p) = w.players.get_mut(&pid) else { return };
    if p.kind != PlayerKind::Ai {
        return;
    }
    p.ai_level = level;
    if w.role == Role::Server {
        let m = level_message(pid, level);
        w.send(m);
    }
}

fn level_message(pid: Pid, level: AiLevel) -> crate::net::Msg {
    crate::net::Msg::ChatMessage { msg: format!("{}{} {}", LEVEL_PREFIX, pid, level.index()), from_id: crate::plugins::TIMER_FROM, to_id: 0xffff }
}

/// Host: every computer player's level, to a player who joined.
pub fn send_levels_to(w: &mut World, c: crate::net::ConnId) {
    let all: Vec<(Pid, AiLevel)> = w.players.values().filter(|p| p.kind == PlayerKind::Ai).map(|p| (p.id, p.ai_level)).collect();
    for (pid, l) in all {
        let m = level_message(pid, l);
        w.send_to(m, &[c], &[]);
    }
}

/// Host: every computer player's level, to everybody (the match starts).
pub fn send_levels(w: &mut World) {
    if w.role != Role::Server {
        return;
    }
    let all: Vec<(Pid, AiLevel)> = w.players.values().filter(|p| p.kind == PlayerKind::Ai).map(|p| (p.id, p.ai_level)).collect();
    for (pid, l) in all {
        let m = level_message(pid, l);
        w.send(m);
    }
}

/// A player's game heard a level (true if it was one).
pub fn client_message(w: &mut World, msg: &str) -> bool {
    let Some(rest) = msg.strip_prefix(LEVEL_PREFIX) else { return false };
    let mut it = rest.split_whitespace();
    if let (Some(Ok(pid)), Some(Ok(l))) = (it.next().map(|v| v.parse::<Pid>()), it.next().map(|v| v.parse::<u8>())) {
        if let Some(p) = w.players.get_mut(&pid) {
            p.ai_level = AiLevel::from_index(l);
        }
    }
    true
}

/// The strategy of the moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// gather, make more workers, wall up
    #[default]
    Build,
    /// strong enough: go and take igloos
    Attack,
    /// enemies at home: everybody back, more snowballers
    Defend,
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
    // (the strategy: no new attacks while building up or defending)
    if ai.mode != Mode::Attack {
        return -30;
    }
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
        if !ai.attacking.contains(&u) && !ai.defenders.contains(&u) && !ai.runners.contains_key(&u) && w.units[u].warmth > 4 {
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
    // (each thinks at its own pace)
    let now = w.now();
    if (now.wrapping_sub(ai.next_think) as i32) < 0 {
        put(w, pid, ai);
        return;
    }
    let level = level_of(w, Some(pid));
    ai.next_think = now.wrapping_add(level.think_ms());
    refresh_units(w, &mut ai);
    choose_mode(w, &mut ai, level);
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
    defend(w, &mut ai, level);
    if level != AiLevel::Easy {
        powerups(w, &mut ai, level);
        switch_classes(w, &mut ai, level);
        walls(w, &mut ai, level);
    }
    put(w, pid, ai);
}

// -- the smarts ----------------------------------------------------------------------

/// Its penguins as they are now (classes change, igloos are won and lost).
fn refresh_units(w: &mut World, ai: &mut Ai) {
    let pid = ai.pid;
    let mut snow = BTreeSet::new();
    let mut workers = Vec::new();
    for ui in 0..w.units.len() {
        if w.igloo_player(ui) != Some(pid) {
            continue;
        }
        if w.units[ui].kind == UnitKind::Snowballer {
            snow.insert(ui);
        } else {
            workers.push(ui);
        }
    }
    ai.attacking.retain(|u| snow.contains(u));
    ai.guarding.retain(|u| snow.contains(u));
    ai.defenders.retain(|u| snow.contains(u));
    ai.runners.retain(|u, _| snow.contains(u));
    ai.units = snow.difference(&ai.guarding).copied().collect();
    // new workers (a class change) go gathering
    let gather = w.ai_cfg.gather.unwrap_or(ResKind::Fish);
    for &wk in &workers {
        if !ai.workers.contains(&wk) && w.units[wk].going_for.is_none() && w.units[wk].build_wall.is_none() {
            w.units[wk].going_for = Some(gather);
        }
    }
    ai.workers = workers;
}

fn warm_snowballers(w: &World, p: Pid) -> usize {
    (0..w.units.len()).filter(|&u| w.igloo_player(u) == Some(p) && w.units[u].kind == UnitKind::Snowballer && w.units[u].warmth >= 5).count()
}

/// My igloos.
fn my_igloos(w: &World, pid: Pid) -> Vec<usize> {
    (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(pid)).collect()
}

/// Enemy snowballers (warm) within `r` tiles of an igloo.
fn threat_at(w: &World, pid: Pid, b: usize, r: i32) -> usize {
    let (bx, by) = (w.buildings[b].x + 1, w.buildings[b].y + 1);
    (0..w.units.len())
        .filter(|&u| {
            let un = &w.units[u];
            un.kind == UnitKind::Snowballer && un.warmth >= 5 && (un.x - bx).abs().max((un.y - by).abs()) <= r && !alliance::friendly(w, w.igloo_player(u), Some(pid))
        })
        .count()
}

/// Build up, attack or defend: by how strong it is against its enemies
/// and what is happening at home.
fn choose_mode(w: &mut World, ai: &mut Ai, level: AiLevel) {
    let pid = ai.pid;
    let mine = warm_snowballers(w, pid) as f64;
    let strongest = w.players.keys().copied().filter(|&p| p != pid && alliance::alive(w, p) && !alliance::friendly(w, Some(p), Some(pid))).map(|p| warm_snowballers(w, p)).max().unwrap_or(0) as f64;
    let home = my_igloos(w, pid);
    let threatened = home.iter().any(|&b| w.buildings[b].jeopardy || threat_at(w, pid, b, 10) >= 3);
    let still_threat = home.iter().any(|&b| w.buildings[b].jeopardy || threat_at(w, pid, b, 12) >= 1);
    // (a strategy holds a while, and attacks go on until clearly too weak)
    let now = w.now();
    let settled = now.wrapping_sub(ai.mode_since) >= 15_000;
    let strong = mine >= 5.0 && mine >= strongest * level.attack_ratio();
    let still_strong = mine >= 4.0 && mine >= strongest * level.attack_ratio() * 0.75;
    let mode = if threatened || (ai.mode == Mode::Defend && still_threat) {
        Mode::Defend
    } else if !settled {
        ai.mode
    } else if strong || (ai.mode == Mode::Attack && still_strong) {
        Mode::Attack
    } else {
        Mode::Build
    };
    if mode != ai.mode {
        ai.mode_since = now;
        if crate::unit::throw_log() {
            println!("AI {} ({}) {:?} -> {:?} (mine {}, strongest {})", w.player_name(pid), level.label(), ai.mode, mode, mine, strongest);
        }
        ai.mode = mode;
    }
}

/// Enemies at home: send snowballers back (into the igloo if it is being
/// taken: that stops the takeover), more on harder levels.
fn defend(w: &mut World, ai: &mut Ai, level: AiLevel) {
    let pid = ai.pid;
    // defenders whose igloo is safe again go back to what they did
    let safe: Vec<usize> = ai.defenders.iter().copied().filter(|&u| {
        let b = w.units[u].igloo;
        !w.buildings[b].jeopardy && threat_at(w, pid, b, 10) == 0
    }).collect();
    for u in safe {
        ai.defenders.remove(&u);
        w.units[u].target_pos = None;
    }
    if ai.mode != Mode::Defend {
        return;
    }
    let per_igloo = match level {
        AiLevel::Easy => 2,
        AiLevel::Normal => 4,
        AiLevel::Hard => 7,
    };
    for b in my_igloos(w, pid) {
        let jeopardy = w.buildings[b].jeopardy;
        let threat = threat_at(w, pid, b, 10);
        if !jeopardy && threat < 3 {
            continue;
        }
        let door = (w.buildings[b].x, w.buildings[b].y);
        let have = ai.defenders.iter().filter(|&&u| w.units[u].igloo == b).count();
        let want = per_igloo.min(threat + 2).saturating_sub(have);
        // the nearest warm snowballers (attackers far away too on Normal / Hard)
        let mut cands: Vec<(i32, usize)> = ai
            .units
            .iter()
            .copied()
            .filter(|u| !ai.defenders.contains(u) && w.units[*u].warmth >= 4)
            .filter(|u| level == AiLevel::Hard || !ai.attacking.contains(u) || jeopardy)
            .map(|u| ((w.units[u].x - door.0).abs().max((w.units[u].y - door.1).abs()), u))
            .collect();
        cands.sort();
        for (_, u) in cands.into_iter().take(want) {
            ai.attacking.remove(&u);
            ai.runners.remove(&u);
            ai.defenders.insert(u);
            let un = &mut w.units[u];
            un.last_tiles.clear();
            if jeopardy {
                un.target_pos = None;
                un.set_go_to_building(Some(b));
            } else {
                un.set_go_to_building(None);
                un.target_pos = Some((door.0 + 1, door.1 + 2));
                un.stray_range = 3;
            }
        }
    }
}

/// Send a snowballer for a power-up lying near (one each).
fn powerups(w: &mut World, ai: &mut Ai, level: AiLevel) {
    // runners that got it (or it is gone)
    let lying = crate::plugins::powerups_on_ground(w);
    let done: Vec<usize> = ai.runners.iter().filter(|(&u, p)| !lying.contains(p) || crate::plugins::power_of(w, u).is_some()).map(|(&u, _)| u).collect();
    for u in done {
        if let Some(p) = ai.runners.remove(&u) {
            if w.units[u].target_pos == Some(p) {
                w.units[u].target_pos = None;
            }
        }
    }
    if ai.mode == Mode::Defend {
        return;
    }
    let reach = if level == AiLevel::Hard { 22 } else { 13 };
    for p in lying {
        if ai.runners.values().any(|&q| q == p) {
            continue;
        }
        let best = ai
            .units
            .iter()
            .copied()
            .filter(|u| !ai.runners.contains_key(u) && !ai.defenders.contains(u) && w.units[*u].warmth >= 5 && crate::plugins::power_of(w, *u).is_none())
            .filter(|u| !matches!(w.units[*u].state, State::InBuilding | State::Entering | State::Frozen))
            .map(|u| ((w.units[u].x - p.0).abs().max((w.units[u].y - p.1).abs()), u))
            .filter(|&(d, _)| d <= reach)
            .min();
        if let Some((_, u)) = best {
            ai.attacking.remove(&u);
            ai.runners.insert(u, p);
            let un = &mut w.units[u];
            un.set_go_to_building(None);
            un.target_pos = Some(p);
            un.stray_range = 0;
            un.last_tiles.clear();
        }
    }
}

/// Keep the mix of snowballers and workers fitting the strategy (a fish
/// a change, like a player's).
fn switch_classes(w: &mut World, ai: &mut Ai, level: AiLevel) {
    let now = w.now();
    if (now.wrapping_sub(ai.next_switch) as i32) < 0 {
        return;
    }
    let snow = ai.units.len() + ai.guarding.len();
    let total = snow + ai.workers.len();
    if total < 4 {
        return;
    }
    let gather_map = w.ai_cfg.gather.is_some();
    let want = match ai.mode {
        Mode::Defend => 0.75,
        Mode::Attack => 0.7,
        Mode::Build => if gather_map { 0.4 } else { 0.5 },
    };
    let share = snow as f64 / total as f64;
    let n = if level == AiLevel::Hard { 2 } else { 1 };
    let pid = ai.pid;
    let mut ids: Vec<usize> = Vec::new();
    if share < want - 0.12 {
        // more snowballers: workers not carrying anything, near home
        ids = ai.workers.iter().copied().filter(|&u| w.units[u].holding.is_none() && w.units[u].build_wall.is_none() && w.units[u].warmth >= 3).take(n).collect();
    } else if share > want + 0.15 && ai.mode == Mode::Build {
        ids = ai
            .units
            .iter()
            .copied()
            .filter(|u| !ai.attacking.contains(u) && !ai.defenders.contains(u) && !ai.runners.contains_key(u) && crate::plugins::power_of(w, *u).is_none() && w.units[*u].warmth >= 3)
            .take(n)
            .collect();
    }
    if ids.is_empty() {
        return;
    }
    let (sent, _poor) = crate::unit::order_switch(w, Some(pid), &ids);
    if sent > 0 {
        ai.next_switch = now.wrapping_add(if level == AiLevel::Hard { 8000 } else { 20_000 });
        if crate::unit::throw_log() {
            println!("AI {} switches {} penguins ({:?}, share {:.2} want {:.2})", w.player_name(pid), sent, ai.mode, share, want);
        }
    }
}

/// A wall section between home and the nearest enemy (Normal: one, Hard:
/// up to three, and ice when there is crystal to spare).
fn walls(w: &mut World, ai: &mut Ai, level: AiLevel) {
    let now = w.now();
    let pid = ai.pid;
    let max = if level == AiLevel::Hard { 3 } else { 1 };
    // (Hard: ice on its finished sections, when not racing for crystal)
    if level == AiLevel::Hard && w.ai_cfg.gather != Some(ResKind::Crystal) && crate::wall::crystal_of(w, pid) >= 2 {
        let mine: Vec<(i32, i32)> = w.walls.iter().filter(|s| s.alive && s.owner == Some(pid) && s.built() && !s.ice && s.upgrade.is_none()).map(|s| (s.x, s.y)).collect();
        if let Some(&t) = mine.first() {
            let walls = crate::wall::upgradable(w, Some(pid), &[t]);
            let workers: Vec<usize> = ai.workers.iter().copied().filter(|&u| w.units[u].holding.is_none()).take(2).collect();
            if !walls.is_empty() && !workers.is_empty() {
                crate::wall::upgrade(w, Some(pid), &walls, &workers);
            }
        }
    }
    if ai.walls_planned >= max || (now.wrapping_sub(ai.next_wall) as i32) < 0 || ai.mode == Mode::Attack {
        return;
    }
    // (not in the first minute)
    if now.wrapping_sub(w.started_at) < 60_000 {
        return;
    }
    let home = my_igloos(w, pid);
    let Some(&b) = home.first() else { return };
    let door = (w.buildings[b].x + 1, w.buildings[b].y + 1);
    // facing the nearest enemy igloo first, then the next nearest, ...
    let mut enemies: Vec<usize> = (0..w.buildings.len()).filter(|&e| w.buildings[e].player.is_some() && !alliance::friendly(w, w.buildings[e].player, Some(pid))).collect();
    enemies.sort_by_key(|&e| (w.buildings[e].x - door.0).abs() + (w.buildings[e].y - door.1).abs());
    let Some(&enemy) = enemies.get(ai.walls_planned as usize % enemies.len().max(1)) else { return };
    let (dx, dy) = (w.buildings[enemy].x - door.0, w.buildings[enemy].y - door.1);
    let (ax, ay) = (dx.abs() as f64, dy.abs() as f64);
    // the way to the enemy, in 8 directions; the wall lies across it
    let dir = if ax > 2.0 * ay { (dx.signum(), 0) } else if ay > 2.0 * ax { (0, dy.signum()) } else { (dx.signum(), dy.signum()) };
    let rot = match dir {
        (_, 0) => 2,
        (0, _) => 0,
        (a, b) if a == b => 3,
        _ => 1,
    };
    let workers: Vec<usize> = {
        let mut v: Vec<(i32, usize)> = ai
            .workers
            .iter()
            .copied()
            .filter(|&u| w.units[u].build_wall.is_none() && w.units[u].warmth >= 4 && !matches!(w.units[u].state, State::InBuilding | State::Entering | State::Frozen))
            .map(|u| ((w.units[u].x - door.0).abs().max((w.units[u].y - door.1).abs()), u))
            .collect();
        v.sort();
        v.into_iter().take(if level == AiLevel::Hard { 4 } else { 3 }).map(|x| x.1).collect()
    };
    if workers.is_empty() {
        return;
    }
    for dist in [7, 8, 9, 6, 10] {
        for side in [0, 1, -1, 2, -2] {
            let perp = (-dir.1, dir.0);
            let c = (door.0 + dir.0 * dist + perp.0 * side, door.1 + dir.1 * dist + perp.1 * side);
            let tiles = crate::wall::piece_tiles(c, rot);
            // (not right next to a wall of its own: a new side)
            let near_own = w.walls.iter().any(|s| s.alive && s.owner == Some(pid) && (s.x - c.0).abs().max((s.y - c.1).abs()) <= 4);
            if !near_own && tiles.iter().all(|&(x, y)| crate::wall::can_place(w, x, y)) {
                let n = crate::wall::plan(w, Some(pid), tiles[0], tiles[tiles.len() - 1], &workers);
                if n > 0 {
                    ai.walls_planned += 1;
                    ai.next_wall = now.wrapping_add(if level == AiLevel::Hard { 45_000 } else { 90_000 });
                    if crate::unit::throw_log() {
                        println!("AI {} plans a wall at {:?} ({} workers)", w.player_name(pid), c, workers.len());
                    }
                    return;
                }
            }
        }
    }
    // (nowhere to put it: try again later)
    ai.next_wall = now.wrapping_add(30_000);
}

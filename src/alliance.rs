//! Alliances: a player offers another an alliance, the other accepts or
//! turns it down.  Accepted, both pay `ALLIANCE_FISH` fish and their
//! teams merge: allies don't throw at each other, can't take each other's
//! igloos, and win together (conquer: their team holds every igloo;
//! gather: the team's crystal reaches the amount times its size).
//!
//! The host decides everything; the players' games hear about it through
//! hidden "chat messages" (see `plugins::TIMER_FROM`), which older
//! versions leave out.

use crate::net::Msg;
use crate::plugins::TIMER_FROM;
use crate::world::*;

/// What an alliance costs each side.
pub const ALLIANCE_FISH: i32 = 10;
/// An offer nobody answers runs out after this long.
pub const OFFER_MS: u32 = 60_000;

const OFFER_PREFIX: &str = "#ally-offer ";
const ANSWER_PREFIX: &str = "#ally-answer ";
const TEAMS_PREFIX: &str = "#ally-teams ";
const OFFERS_PREFIX: &str = "#ally-offers ";
const NOTE_PREFIX: &str = "#ally-note ";

/// Are these two players allies?
pub fn allied(w: &World, a: Option<Pid>, b: Option<Pid>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if a != b => match (w.teams.get(&a), w.teams.get(&b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        _ => false,
    }
}

/// On the same side: the same player or allies.
pub fn friendly(w: &World, a: Option<Pid>, b: Option<Pid>) -> bool {
    a == b || allied(w, a, b)
}

/// A player and its allies.
pub fn team_of(w: &World, p: Pid) -> Vec<Pid> {
    match w.teams.get(&p) {
        Some(t) => w.teams.iter().filter(|(_, x)| *x == t).map(|(&q, _)| q).collect(),
        None => vec![p],
    }
}

/// Still in the game (has an igloo).
pub fn alive(w: &World, p: Pid) -> bool {
    w.buildings.iter().any(|b| b.player == Some(p))
}

/// All the fish in a player's igloos.
pub fn fish_of(w: &World, p: Pid) -> i32 {
    (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(p)).map(|b| w.num_storage(b, ResKind::Fish)).sum()
}

/// Take fish from a player's igloos (the fullest first).
fn pay(w: &mut World, p: Pid, mut n: i32) {
    let mut igloos: Vec<usize> = (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(p)).collect();
    igloos.sort_by_key(|&b| -w.num_storage(b, ResKind::Fish));
    for b in igloos {
        let take = w.num_storage(b, ResKind::Fish).min(n);
        if take > 0 {
            w.add_storage(b, ResKind::Fish, -take);
            n -= take;
        }
        if n == 0 {
            break;
        }
    }
}

/// Why `from` can't offer `to` an alliance right now (None: it can).
pub fn cannot_offer(w: &World, from: Pid, to: Pid) -> Option<String> {
    if from == to || !w.players.contains_key(&to) {
        return Some("No such player".into());
    }
    if !alive(w, from) || !alive(w, to) {
        return Some(format!("{} is out of the game", w.player_name(if alive(w, from) { to } else { from })));
    }
    if allied(w, Some(from), Some(to)) {
        return Some(format!("You are already allied with {}", w.player_name(to)));
    }
    if w.offers.iter().any(|o| o.0 == from && o.1 == to) {
        return Some(format!("You already offered {} an alliance", w.player_name(to)));
    }
    if fish_of(w, from) < ALLIANCE_FISH {
        return Some(format!("An alliance costs {} fish (you have {})", ALLIANCE_FISH, fish_of(w, from)));
    }
    None
}

// -- the host --------------------------------------------------------------

/// `from` offers `to` an alliance (host / single player).
pub fn offer(w: &mut World, from: Pid, to: Pid) {
    if let Some(why) = cannot_offer(w, from, to) {
        note(w, from, &why);
        return;
    }
    // an offer back the other way is as good as a yes
    if w.offers.iter().any(|o| o.0 == to && o.1 == from) {
        answer(w, to, from, true);
        return;
    }
    let now = w.now();
    w.offers.push((from, to, now));
    let (a, b) = (w.player_name(from), w.player_name(to));
    announce(w, &format!("{} offers {} an alliance", a, b));
    send_offers(w);
}

/// `to` answers `from`'s offer (host / single player).
pub fn answer(w: &mut World, from: Pid, to: Pid, accept: bool) {
    let Some(i) = w.offers.iter().position(|o| o.0 == from && o.1 == to) else { return };
    w.offers.remove(i);
    let (a, b) = (w.player_name(from), w.player_name(to));
    if !accept {
        announce(w, &format!("{} turned down {}'s alliance", b, a));
        send_offers(w);
        return;
    }
    for p in [from, to] {
        if fish_of(w, p) < ALLIANCE_FISH || !alive(w, p) {
            let who = w.player_name(p);
            announce(w, &format!("No alliance between {} and {}: {} can't pay {} fish", a, b, who, ALLIANCE_FISH));
            send_offers(w);
            return;
        }
    }
    pay(w, from, ALLIANCE_FISH);
    pay(w, to, ALLIANCE_FISH);
    // the two teams become one
    let t = w.teams.get(&from).or_else(|| w.teams.get(&to)).copied().unwrap_or_else(|| {
        w.next_team += 1;
        w.next_team
    });
    let members: Vec<Pid> = team_of(w, from).into_iter().chain(team_of(w, to)).collect();
    for p in &members {
        w.teams.insert(*p, t);
    }
    // offers inside the new team are moot
    let teams = w.teams.clone();
    w.offers.retain(|o| teams.get(&o.0).is_none() || teams.get(&o.0) != teams.get(&o.1));
    let names: Vec<String> = members.iter().map(|&p| w.player_name(p)).collect();
    if members.len() > 2 {
        announce(w, &format!("{} and {} are now allies: the alliance is {}", a, b, names.join(", ")));
    } else {
        announce(w, &format!("{} and {} are now allies", a, b));
    }
    send_teams(w);
    send_offers(w);
    // their penguins stop going for each other
    for u in 0..w.units.len() {
        if let Some(t) = w.units[u].targeted_unit {
            if allied(w, w.igloo_player(u), w.igloo_player(t)) {
                w.units[u].targeted_unit = None;
            }
        }
    }
    crate::plugins::check_victory(w);
}

/// Offers run out (host, every frame).
pub fn tick(w: &mut World) {
    let now = w.now();
    let old: Vec<(Pid, Pid)> = w.offers.iter().filter(|o| now.wrapping_sub(o.2) >= OFFER_MS).map(|o| (o.0, o.1)).collect();
    if old.is_empty() {
        return;
    }
    w.offers.retain(|o| now.wrapping_sub(o.2) < OFFER_MS);
    for (a, b) in old {
        let (an, bn) = (w.player_name(a), w.player_name(b));
        announce(w, &format!("{}'s alliance offer to {} ran out", an, bn));
    }
    send_offers(w);
}

/// A line in everybody's chat.
fn announce(w: &mut World, text: &str) {
    println!("ALLIANCE {}", text);
    w.notice(text);
    if w.role == Role::Server {
        hidden(w, format!("{}{}", NOTE_PREFIX, text), None);
    }
}

/// A line for one player only (why their offer didn't go).
fn note(w: &mut World, p: Pid, text: &str) {
    if Some(p) == w.me {
        w.notice(text);
    } else if let Some(c) = w.players.get(&p).and_then(|pl| pl.connection) {
        hidden(w, format!("{}{}", NOTE_PREFIX, text), Some(c));
    }
}

fn hidden(w: &mut World, msg: String, to: Option<crate::net::ConnId>) {
    if w.role != Role::Server {
        return;
    }
    let m = Msg::ChatMessage { msg, from_id: TIMER_FROM, to_id: 0xffff };
    match to {
        Some(c) => w.send_to(m, &[c], &[]),
        None => w.send(m),
    }
}

fn teams_line(w: &World) -> String {
    let t: Vec<String> = w.teams.iter().map(|(p, t)| format!("{}:{}", p, t)).collect();
    format!("{}{}", TEAMS_PREFIX, t.join(","))
}

fn offers_line(w: &World) -> String {
    let now = w.now();
    let o: Vec<String> = w.offers.iter().map(|o| format!("{}>{}>{}", o.0, o.1, OFFER_MS.saturating_sub(now.wrapping_sub(o.2)))).collect();
    format!("{}{}", OFFERS_PREFIX, o.join(","))
}

fn send_teams(w: &mut World) {
    let l = teams_line(w);
    hidden(w, l, None);
}

fn send_offers(w: &mut World) {
    let l = offers_line(w);
    hidden(w, l, None);
}

/// A player joined during the game: tell them how things stand.
pub fn send_state_to(w: &mut World, c: crate::net::ConnId) {
    let (t, o) = (teams_line(w), offers_line(w));
    hidden(w, t, Some(c));
    hidden(w, o, Some(c));
}

/// Host: a player's request (true if it was one).
pub fn host_message(w: &mut World, msg: &str, from: Option<Pid>) -> bool {
    let Some(from) = from else { return msg.starts_with("#ally-") };
    if let Some(rest) = msg.strip_prefix(OFFER_PREFIX) {
        if let Ok(to) = rest.trim().parse::<Pid>() {
            offer(w, from, to);
        }
        return true;
    }
    if let Some(rest) = msg.strip_prefix(ANSWER_PREFIX) {
        let mut it = rest.split(' ');
        if let (Some(Ok(by)), Some(yes)) = (it.next().map(|v| v.parse::<Pid>()), it.next()) {
            answer(w, by, from, yes == "1");
        }
        return true;
    }
    false
}

// -- the players' games ----------------------------------------------------

/// Ask for an alliance with `to` (any computer).
pub fn request_offer(w: &mut World, to: Pid) {
    let Some(me) = w.me else { return };
    if w.is_server() {
        offer(w, me, to);
    } else if let Some(why) = cannot_offer(w, me, to) {
        w.notice(&why);
    } else {
        w.send(Msg::ChatMessage { msg: format!("{}{}", OFFER_PREFIX, to), from_id: TIMER_FROM, to_id: 0xffff });
    }
}

/// Answer `from`'s offer to me (any computer).
pub fn request_answer(w: &mut World, from: Pid, accept: bool) {
    let Some(me) = w.me else { return };
    if w.is_server() {
        answer(w, from, me, accept);
    } else {
        w.send(Msg::ChatMessage { msg: format!("{}{} {}", ANSWER_PREFIX, from, accept as u8), from_id: TIMER_FROM, to_id: 0xffff });
    }
}

/// A player's game hears from the host (true if it was about alliances).
pub fn client_message(w: &mut World, msg: &str) -> bool {
    if let Some(rest) = msg.strip_prefix(NOTE_PREFIX) {
        w.notice(rest);
        return true;
    }
    if let Some(rest) = msg.strip_prefix(TEAMS_PREFIX) {
        w.teams.clear();
        for part in rest.split(',').filter(|s| !s.is_empty()) {
            if let Some((p, t)) = part.split_once(':') {
                if let (Ok(p), Ok(t)) = (p.parse::<Pid>(), t.parse::<u32>()) {
                    w.teams.insert(p, t);
                }
            }
        }
        return true;
    }
    if let Some(rest) = msg.strip_prefix(OFFERS_PREFIX) {
        let now = w.now();
        w.offers.clear();
        for part in rest.split(',').filter(|s| !s.is_empty()) {
            let v: Vec<&str> = part.split('>').collect();
            if let [a, b, left] = v[..] {
                if let (Ok(a), Ok(b), Ok(left)) = (a.parse::<Pid>(), b.parse::<Pid>(), left.parse::<u32>()) {
                    // (kept so that it runs out here when it does at the host)
                    w.offers.push((a, b, now.wrapping_sub(OFFER_MS.saturating_sub(left))));
                }
            }
        }
        return true;
    }
    false
}

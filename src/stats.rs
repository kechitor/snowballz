//! Match statistics for the after-match screen.
//!
//! The host counts (penguins frozen, snowballs thrown and hits, loads
//! gathered, igloos taken, walls built, power-ups picked up) and sends them
//! to everybody just before it announces the winner, in a hidden "chat
//! message" (see `plugins::TIMER_FROM`).

use crate::net::Msg;
use crate::plugins::TIMER_FROM;
use crate::world::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// enemy penguins this player's snowballs froze
    pub frozen: u32,
    /// this player's penguins frozen
    pub lost: u32,
    pub thrown: u32,
    /// direct hits / near misses on enemy penguins
    pub hits: u32,
    pub near: u32,
    pub fish: u32,
    pub crystal: u32,
    /// igloos taken from another player
    pub igloos: u32,
    /// wall tiles finished
    pub walls: u32,
    pub powerups: u32,
}

const FIELDS: usize = 10;

impl Stats {
    fn to_list(self) -> [u32; FIELDS] {
        [self.frozen, self.lost, self.thrown, self.hits, self.near, self.fish, self.crystal, self.igloos, self.walls, self.powerups]
    }

    fn from_list(v: &[u32]) -> Stats {
        let g = |i: usize| v.get(i).copied().unwrap_or(0);
        Stats { frozen: g(0), lost: g(1), thrown: g(2), hits: g(3), near: g(4), fish: g(5), crystal: g(6), igloos: g(7), walls: g(8), powerups: g(9) }
    }

    /// Direct hits per throw, in percent.
    pub fn accuracy(&self) -> Option<u32> {
        (self.thrown > 0).then(|| (self.hits * 100 + self.thrown / 2) / self.thrown)
    }
}

/// Count something for `p` (host and single player only; the players'
/// games get the totals at the end).
pub fn add(w: &mut World, p: Option<Pid>, f: impl FnOnce(&mut Stats)) {
    if !w.is_server() || !w.started {
        return;
    }
    if let Some(p) = p {
        f(w.stats.entry(p).or_default());
    }
}

const PREFIX: &str = "#stats ";

/// Host: everybody's totals (and how long the match ran) to everybody.
pub fn broadcast(w: &mut World) {
    if w.role != Role::Server {
        return;
    }
    let ms = w.now().wrapping_sub(w.started_at);
    let mut parts = vec![ms.to_string()];
    for (p, s) in &w.stats {
        let v: Vec<String> = s.to_list().iter().map(|n| n.to_string()).collect();
        parts.push(format!("{}:{}", p, v.join(",")));
    }
    let m = Msg::ChatMessage { msg: format!("{}{}", PREFIX, parts.join(" ")), from_id: TIMER_FROM, to_id: 0xffff };
    w.send(m);
}

/// A player's game heard the totals (true if it was them).
pub fn client_message(w: &mut World, msg: &str) -> bool {
    let Some(rest) = msg.strip_prefix(PREFIX) else { return false };
    let mut it = rest.split(' ');
    if let Some(ms) = it.next().and_then(|v| v.parse::<u32>().ok()) {
        w.match_ms = Some(ms);
    }
    w.stats.clear();
    for part in it {
        let Some((p, v)) = part.split_once(':') else { continue };
        let Ok(p) = p.parse::<Pid>() else { continue };
        let nums: Vec<u32> = v.split(',').filter_map(|n| n.parse().ok()).collect();
        w.stats.insert(p, Stats::from_list(&nums));
    }
    true
}

/// How long the match ran (ms).
pub fn match_ms(w: &World) -> u32 {
    w.match_ms.unwrap_or_else(|| w.now().wrapping_sub(w.started_at))
}

pub fn format_time(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

// -- the after-match screen -------------------------------------------------------

const COLUMNS: [&str; 10] = ["Frozen", "Lost", "Thrown", "Hit %", "Fish", "Crystal", "Igloos", "Walls", "Power-ups", ""];

/// Draw the statistics panel over the winner / loser picture (ui mode).
/// `alpha` fades it in.
pub fn draw_screen(w: &World, won: bool, alpha: f32) {
    use crate::gfx;
    gfx::begin_ui();
    let (sw, sh) = gfx::screen_size();
    let (sw, sh) = (sw as f64, sh as f64);
    let k = (sh / 720.0).clamp(0.8, 2.0);
    let font = |px: f64| w.chat_font.with_size((px * k).round().max(8.0) as u32);
    let (title_f, head_f, row_f, small_f) = (font(24.0), font(17.0), font(20.0), font(15.0));
    // the players: winners first, then by penguins frozen
    let mut players: Vec<&Player> = w.players.values().collect();
    players.sort_by_key(|p| (p.victory != Some(true), std::cmp::Reverse(w.stats.get(&p.id).map(|s| s.frozen).unwrap_or(0)), p.id));
    let row_h = 34.0 * k;
    let name_w = 230.0 * k;
    let col_w = 82.0 * k;
    let cols = COLUMNS.len() - 1;
    let pw = (name_w + col_w * cols as f64 + 40.0 * k).min(sw - 20.0);
    let ph = 70.0 * k + row_h * (players.len() as f64 + 1.0) + 50.0 * k;
    // (under the picture's "Winner" / "Loser")
    let (px, ptop) = ((sw - pw) / 2.0, sh * 0.72);
    // (y counts from the bottom in ui mode)
    let a = alpha;
    gfx::fill_rect(px, ptop - ph, pw, ph, Some([0.05, 0.07, 0.15, 0.82 * a]));
    let white = [1.0, 1.0, 1.0, a];
    let dim = [0.72, 0.78, 0.92, a];
    let map = w.config.get("map", "name").unwrap_or(&w.map_name).to_string();
    let sub = format!("{}  -  {}  -  match time {}", map, if won { "you won" } else { "you lost" }, format_time(match_ms(w)));
    let (stw, sth) = title_f.size_of(&sub);
    let mut y = ptop - 14.0 * k - sth as f64;
    title_f.render((sw - stw as f64) / 2.0, y, &sub, if won { [1.0, 0.88, 0.45, a] } else { white });
    // the header
    y -= row_h + 6.0 * k;
    let x0 = px + 20.0 * k;
    head_f.render(x0, y, "Player", dim);
    for (c, label) in COLUMNS.iter().take(cols).enumerate() {
        let (lw, _) = head_f.size_of(label);
        head_f.render(x0 + name_w + col_w * c as f64 + (col_w - lw as f64) / 2.0, y, label, dim);
    }
    gfx::fill_rect(x0, y - 8.0 * k, pw - 40.0 * k, 2.0, Some([0.6, 0.65, 0.8, 0.6 * a]));
    for p in players {
        y -= row_h;
        if Some(p.id) == w.me {
            gfx::fill_rect(x0 - 8.0 * k, y - 7.0 * k, pw - 24.0 * k, row_h - 2.0, Some([1.0, 1.0, 1.0, 0.09 * a]));
        }
        let c = p.glcolor;
        let sq = 16.0 * k;
        gfx::fill_rect(x0, y + 2.0 * k, sq, sq, Some([c[0], c[1], c[2], a]));
        let mut name = p.name.clone();
        if p.victory == Some(true) {
            name.push_str("  (won)");
        }
        row_f.render(x0 + sq + 10.0 * k, y, &name, if p.victory == Some(true) { [1.0, 0.88, 0.45, a] } else { white });
        let st = w.stats.get(&p.id).copied().unwrap_or_default();
        let acc = st.accuracy().map(|v| format!("{}%", v)).unwrap_or_else(|| "-".into());
        let vals = [st.frozen.to_string(), st.lost.to_string(), st.thrown.to_string(), acc, st.fish.to_string(), st.crystal.to_string(), st.igloos.to_string(), st.walls.to_string(), st.powerups.to_string()];
        for (ci, v) in vals.iter().enumerate() {
            let (vw, _) = row_f.size_of(v);
            row_f.render(x0 + name_w + col_w * ci as f64 + (col_w - vw as f64) / 2.0, y, v, white);
        }
    }
    let note = "Frozen: enemy penguins your snowballs froze.  Hit %: direct hits per snowball thrown.  Walls: wall tiles built.";
    let (nw, _) = small_f.size_of(note);
    small_f.render((sw - nw as f64) / 2.0, ptop - ph + 14.0 * k, note, [0.6, 0.66, 0.8, a]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_go_through_the_message_and_back() {
        let s = Stats { frozen: 4, lost: 2, thrown: 30, hits: 10, near: 5, fish: 12, crystal: 3, igloos: 1, walls: 16, powerups: 2 };
        assert_eq!(Stats::from_list(&s.to_list()), s);
        assert_eq!(s.accuracy(), Some(33));
        assert_eq!(Stats::default().accuracy(), None);
        assert_eq!(format_time(125_000), "2:05");
    }
}

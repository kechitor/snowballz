//! Alerts.  When the player's igloo is being taken, or their penguins are
//! hit somewhere they aren't looking, an alert shows (top left, with a
//! warning sound): clicking it, or pressing J, moves the view there.
//! Every frame each player's own game looks whether its igloos went into
//! jeopardy; snowballs report hits as they land.

use crate::sfx;
use crate::world::*;

/// How long an alert stays up (ms, real time).
pub const SHOW_MS: u32 = 7000;
/// J jumps to the last alert this long after it.
pub const JUMP_MS: u32 = 30_000;
/// No new "under attack" alert this soon after the last one near it ...
const ATTACK_QUIET_MS: u32 = 12_000;
/// ... (near: this many tiles)
const ATTACK_NEAR: i32 = 12;
/// The alert sounds don't repeat sooner than this.
const SOUND_QUIET_MS: u32 = 5000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertKind {
    Attack,
    Igloo,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub kind: AlertKind,
    pub x: i32,
    pub y: i32,
    /// when (real time)
    pub at: u32,
}

impl Alert {
    pub fn text(&self) -> &'static str {
        match self.kind {
            AlertKind::Attack => "Your penguins are under attack!",
            AlertKind::Igloo => "Your igloo is being taken!",
        }
    }
}

/// What the alerts remember from the last frame.
#[derive(Default)]
pub struct Watch {
    /// per igloo: in jeopardy
    igloos: Vec<bool>,
    pub latest: Option<Alert>,
    last_attack: Option<Alert>,
    sound_at: [Option<u32>; 2],
}

/// Once a frame (every game: the host's and the players').
pub fn tick(w: &mut World) {
    if !w.started {
        return;
    }
    // my igloos in jeopardy
    let jeopardy: Vec<bool> = w.buildings.iter().map(|b| b.jeopardy).collect();
    if w.watch.igloos.len() == jeopardy.len() {
        for (bi, &j) in jeopardy.iter().enumerate() {
            if j && !w.watch.igloos[bi] && w.me.is_some() && w.buildings[bi].player == w.me {
                let (x, y) = (w.buildings[bi].x, w.buildings[bi].y);
                raise(w, AlertKind::Igloo, x, y);
            }
        }
    }
    w.watch.igloos = jeopardy;
}

/// A snowball landed at tile (x, y); `mine_hit`: one of my penguins was
/// hit.
pub fn snowball_landed(w: &mut World, x: i32, y: i32, mine_hit: bool) {
    if mine_hit && !in_view(w, x, y) {
        let now = crate::sdl::ticks();
        let quiet = w.watch.last_attack.as_ref().is_some_and(|a| now.wrapping_sub(a.at) < ATTACK_QUIET_MS && (a.x - x).abs().max((a.y - y).abs()) <= ATTACK_NEAR);
        if !quiet {
            raise(w, AlertKind::Attack, x, y);
        }
    }
}

fn in_view(w: &World, x: i32, y: i32) -> bool {
    let (px, py) = ((x * 32 + 16) as f64, (y * 32 + 16) as f64);
    let v = &w.view;
    px >= v.x && px <= v.x + v.scale_w() as f64 && py >= v.y && py <= v.y + v.scale_h() as f64
}

fn raise(w: &mut World, kind: AlertKind, x: i32, y: i32) {
    let now = crate::sdl::ticks();
    let a = Alert { kind, x, y, at: now };
    if kind == AlertKind::Attack {
        w.watch.last_attack = Some(a.clone());
    }
    w.watch.latest = Some(a);
    let k = kind as usize;
    if w.watch.sound_at[k].is_none_or(|t| now.wrapping_sub(t) >= SOUND_QUIET_MS) {
        w.watch.sound_at[k] = Some(now);
        sfx::play(if kind == AlertKind::Igloo { "alert_igloo" } else { "alert_attack" }, 0.8, 0.0);
    }
}

/// The alert showing (if any).
pub fn showing(w: &World) -> Option<&Alert> {
    w.watch.latest.as_ref().filter(|a| crate::sdl::ticks().wrapping_sub(a.at) < SHOW_MS)
}

/// J, or a click on the alert: look where it happened.
pub fn jump(w: &mut World) -> bool {
    let Some(a) = w.watch.latest.clone() else { return false };
    if crate::sdl::ticks().wrapping_sub(a.at) > JUMP_MS {
        return false;
    }
    crate::ctrl::look_at(w, (a.x * 32 + 16) as f64, (a.y * 32 + 16) as f64);
    // (shown answered: it goes away)
    if let Some(l) = w.watch.latest.as_mut() {
        l.at = l.at.wrapping_sub(SHOW_MS);
    }
    true
}

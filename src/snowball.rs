//! Snowballs in flight (snowball.py).

use crate::effects::SnowParticles;
use crate::settings;
use crate::unit;
use crate::world::*;

pub struct Snowball {
    pub end_tile: usize,
    pub start_tile: Option<usize>,
    /// the penguin that threw it (server only)
    pub penguin: Option<usize>,
    /// the penguin it was thrown at (server only)
    pub target: Option<usize>,
    /// (for the throw log: target walking, distance, tiles led)
    pub info: (bool, i32, i32, i32),
    /// (range power-up) it follows its target and lands on it; the spot on
    /// the target it lands on
    homing: Option<(f64, f64)>,
    pub start_x: f64,
    pub start_y: f64,
    pub end_x: f64,
    pub end_y: f64,
    dist_x: f64,
    dist_y: f64,
    dist: f64,
    total_time: f64,
    height: f64,
    start_time: u32,
    pub x: f64,
    pub y: f64,
    pub shadow_x: f64,
    pub shadow_y: f64,
}

const SPEED: f64 = 0.15;
/// Throws this close (tiles) are never off ...
const SURE_AIM_TILES: f64 = 3.0;
/// Further throws are on target this often (the rest land next to the
/// target: a near miss) ...
const ON_TARGET: f64 = 0.45;
/// ... and from the highest ground this share of the others is too (less
/// on lower ground).
const HIGH_AIM_MAX: f64 = 0.5;

impl Snowball {
    /// Throw at a penguin: aim where it will be when the snowball lands
    /// (more and more ahead the further away it is, not at all up close),
    /// with some inaccuracy from far away.  The range power-up aims
    /// perfectly: its snowballs follow the target and land on it.
    pub fn throw_at_target(w: &mut World, ui: usize, target: usize) -> Snowball {
        let (px, py) = (w.units[ui].x, w.units[ui].y);
        let (tx, ty) = lead_target(w, ui, target);
        let t0 = (w.units[target].x, w.units[target].y);
        let info = (
            w.units[target].moving,
            (t0.0 - px).abs().max((t0.1 - py).abs()),
            (tx - t0.0).abs().max((ty - t0.1).abs()),
            (w.elevation(px, py) - w.elevation(t0.0, t0.1)).round() as i32,
        );
        let sharp = crate::plugins::power_of(w, ui) == Some(crate::plugins::PowerKind::Range);
        // up close nobody misses; from higher ground "up close" reaches
        // further, and further away the aim is often spot on too
        let here = w.elevation(px, py);
        let there = w.elevation(w.units[target].x, w.units[target].y);
        let lift = unit::height_bonus(here, there).max(0.0);
        let sure = SURE_AIM_TILES + lift;
        let d = ((((tx - px) * (tx - px) + (ty - py) * (ty - py)) as f64).sqrt()).floor();
        // further away a throw is on target this often (more from higher
        // ground); otherwise it lands on a tile next to the target
        // (computer players: by their level)
        let level = crate::ai::level_of(w, w.igloo_player(ui)).aim();
        let on_target = ((ON_TARGET + (1.0 - ON_TARGET) * lift / unit::HEIGHT_BONUS_MAX * HIGH_AIM_MAX) * level).min(0.95);
        let steady = w.rng.random() < on_target;
        let (mut ex, mut ey) = if sharp || d <= sure || steady {
            (tx, ty)
        } else {
            loop {
                let p = (w.rng.randint(tx - 1, tx + 1), w.rng.randint(ty - 1, ty + 1));
                if p != (tx, ty) {
                    break p;
                }
            }
        };
        if w.tile_index(ex, ey).is_none() {
            ex = tx;
            ey = ty;
        }
        // an enemy wall just in front of the target catches it
        let mut sharp = sharp;
        if let Some(wi) = crate::wall::cover(w, (px, py), (ex, ey), w.igloo_player(ui)) {
            ex = w.walls[wi].x;
            ey = w.walls[wi].y;
            sharp = false;
        }
        let end_tile = w.tile_index(ex, ey).unwrap();
        let spot = (w.rng.randint(0, 20) as f64, w.rng.randint(0, 20) as f64);
        let end_x = (ex * 32) as f64 + spot.0;
        let end_y = (ey * 32) as f64 + spot.1 - w.tiles[end_tile].elevation * ELEV_MULT;
        let start_tile = w.tile_index(px, py).unwrap();
        let sx = (px * 32) as f64;
        let sy = (py * 32) as f64 - w.tiles[start_tile].elevation * ELEV_MULT;
        let mut s = Snowball::new(w, sx, sy, end_x, end_y, end_tile, Some(start_tile), Some(ui));
        s.target = Some(target);
        if sharp {
            s.homing = Some(spot);
        }
        s.info = info;
        s
    }

    /// Throw at a tile (a wall section).
    pub fn throw_at_tile(w: &mut World, ui: usize, (ex, ey): (i32, i32)) -> Snowball {
        let (px, py) = (w.units[ui].x, w.units[ui].y);
        let end_tile = w.tile_index(ex, ey).unwrap();
        let spot = (w.rng.randint(6, 14) as f64, w.rng.randint(6, 14) as f64);
        let end_x = (ex * 32) as f64 + spot.0;
        let end_y = (ey * 32) as f64 + spot.1 - w.tiles[end_tile].elevation * ELEV_MULT;
        let start_tile = w.tile_index(px, py).unwrap();
        let sx = (px * 32) as f64;
        let sy = (py * 32) as f64 - w.tiles[start_tile].elevation * ELEV_MULT;
        Snowball::new(w, sx, sy, end_x, end_y, end_tile, Some(start_tile), Some(ui))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(w: &World, start_x: f64, start_y: f64, end_x: f64, end_y: f64, end_tile: usize, start_tile: Option<usize>, penguin: Option<usize>) -> Snowball {
        let dist_x = end_x - start_x;
        let dist_y = end_y - start_y;
        let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
        Snowball {
            end_tile,
            start_tile,
            penguin,
            target: None,
            info: (false, 0, 0, 0),
            homing: None,
            start_x,
            start_y,
            end_x,
            end_y,
            dist_x,
            dist_y,
            dist,
            total_time: dist / SPEED,
            height: ((dist / 2.0) / 10.0).powi(2),
            start_time: w.now(),
            x: start_x,
            y: start_y,
            shadow_x: start_x,
            shadow_y: start_y,
        }
    }
}

/// No leading up to this distance (tiles) ...
const LEAD_FROM: f64 = 3.0;
/// ... full leading from this one.
const LEAD_FULL: f64 = 6.0;

/// The tile to throw at so the snowball meets the target there: a walking
/// penguin keeps heading where it is going (the place it was sent to, the
/// igloo it is walking into, or straight on) while the snowball flies.
pub fn lead_target(w: &World, ui: usize, target: usize) -> (i32, i32) {
    let (px, py) = (w.units[ui].x, w.units[ui].y);
    let t = &w.units[target];
    let (tx, ty) = (t.x, t.y);
    let dist = |a: (i32, i32)| (((a.0 - px) * (a.0 - px) + (a.1 - py) * (a.1 - py)) as f64).sqrt();
    let lead = ((dist((tx, ty)) - LEAD_FROM) / (LEAD_FULL - LEAD_FROM)).clamp(0.0, 1.0);
    if lead == 0.0 || !t.moving || t.direction == (0, 0) || !matches!(t.state, State::Stray | State::Walking) {
        return (tx, ty);
    }
    // where is it going?
    let dest = if let Some(b) = t.go_to_building() {
        Some((w.buildings[b].x, w.buildings[b].y))
    } else if let Some(tp) = t.target_pos {
        let r = t.stray_range;
        if (tp.0 - tx).abs() <= r && (tp.1 - ty).abs() <= r {
            // it is there already, just milling around
            return (tx, ty);
        }
        Some(tp)
    } else {
        None
    };
    // an empty-handed snowballer stops on the snow to make a snowball
    let on_snow = w.tile(tx, ty).map(|tl| tl.kind) == Some(TileKind::Snow);
    if t.kind == UnitKind::Snowballer && t.snowballs == 0 && !t.run_mode && on_snow && t.warmth >= 5 {
        return (tx, ty);
    }
    let ws = unit::walk_speed(w, target).max(100.0);
    // (its tile is where the current step ends; the next step starts then)
    let left = (ws - w.now().wrapping_sub(t.counter) as f64).max(0.0);
    let walk = |steps: i32| -> (i32, i32) {
        let mut p = (tx, ty);
        for _ in 0..steps {
            let d = match dest {
                Some(d) if d == p => break,
                Some(d) => ((d.0 - p.0).signum(), (d.1 - p.1).signum()),
                None => t.direction,
            };
            p = (p.0 + d.0, p.1 + d.1);
        }
        p
    };
    let mut aim = (tx, ty);
    for _ in 0..3 {
        let flight = dist(aim) * 32.0 / SPEED;
        let steps = if flight < left { 0.0 } else { 1.0 + ((flight - left) / ws).floor() };
        aim = walk((steps * lead).round() as i32);
    }
    if w.tile_index(aim.0, aim.1).is_none() {
        return (tx, ty);
    }
    aim
}

/// Move snowball `i`; true when it has landed (it is then removed by the
/// caller).
pub fn move_snowball(w: &mut World, i: usize) -> bool {
    let now = w.now();
    // a homing snowball heads for where its target is now
    let follow = match (w.snowballs[i].homing, w.snowballs[i].target) {
        (Some(spot), Some(t)) if w.units[t].warmth > 0 && w.units[t].state != State::InBuilding => {
            let (x, y) = w.units[t].sprite_xy(crate::gfx::time());
            let ti = w.tile_index(w.units[t].x, w.units[t].y);
            Some((x + spot.0, y + spot.1, ti))
        }
        _ => None,
    };
    let s = &mut w.snowballs[i];
    if let Some((x, y, ti)) = follow {
        s.end_x = x;
        s.end_y = y;
        s.dist_x = x - s.start_x;
        s.dist_y = y - s.start_y;
        if let Some(ti) = ti {
            s.end_tile = ti;
        }
    }
    let td = now.wrapping_sub(s.start_time) as f64;
    if s.total_time > 0.0 {
        // (the same flight as before, by the share of the flight time, so
        // the end point can move)
        let f = (td / s.total_time).min(1.0);
        let mx = f * s.dist_x;
        let my = f * s.dist_y;
        let offset = s.height * (1.0 - (2.0 * f - 1.0).powi(2));
        s.x = s.start_x + mx;
        s.y = s.start_y + my + 8.0 - offset / 2.0;
        s.shadow_x = s.start_x + mx;
        s.shadow_y = s.start_y + my;
    }
    if td < s.total_time {
        return false;
    }
    // landed
    let (end_tile, start_tile, penguin, target, info) = (s.end_tile, s.start_tile, s.penguin, s.target, s.info);
    let mut target_hit = 0;
    let (sx, sy, dx, dy, dist) = (s.x, s.y, s.dist_x, s.dist_y, s.dist);
    let mut hit = 1;
    // an enemy wall catches it (and loses a bit); snowballs fly over
    // their own side's walls
    let thrower = penguin.map(|p| w.igloo_player(p));
    let on_wall = w.tiles[end_tile].wall.filter(|&i| w.walls[i].built() && thrower.is_none_or(|t| !crate::alliance::friendly(w, w.walls[i].owner, t)));
    if let Some(wi) = on_wall {
        hit = 2;
        if w.is_server() && thrower.is_some() {
            crate::wall::damage(w, wi, 1);
        }
        if unit::throw_log() {
            println!("LAND-WALL wall={} hp={}", wi, w.walls[wi].hp);
        }
    } else if w.tiles[end_tile].resource.is_some() {
        hit = 0;
    } else {
        let (tx, ty) = (w.tiles[end_tile].x, w.tiles[end_tile].y);
        let mut mine_hit = false;
        for u in w.units_around(tx, ty) {
            if w.units[u].warmth > 0 && w.units[u].state != State::InBuilding {
                let v = unit::detect_hit(w, u, end_tile, start_tile, penguin);
                if Some(u) == target {
                    target_hit = v;
                }
                if v == 2 {
                    hit = 2;
                }
                if v > 0 && w.me.is_some() && w.igloo_player(u) == w.me && penguin.is_none_or(|p| w.igloo_player(p) != w.me) {
                    mine_hit = true;
                }
            }
        }
        crate::alerts::snowball_landed(w, tx, ty, mine_hit);
    }
    if let (Some(p), Some(_)) = (penguin, target) {
        if unit::throw_log() {
            println!("LAND unit={} power={:?} target_hit={} moving={} dist={} led={} height={} tstate={:?} twarm={}", p, crate::plugins::power_of(w, p), target_hit, info.0, info.1, info.2, info.3, target.map(|t| w.units[t].state), target.map(|t| w.units[t].warmth).unwrap_or(0));
        }
    }
    let detail = settings::option_int("snowballdetail");
    if hit != 2 && detail >= 2 {
        if w.tiles[end_tile].kind == TileKind::Snow {
            w.imprints.snowball.add(sx, sy, [1.0; 3], now);
        } else {
            w.imprints.snowball_ice.add(sx, sy, [1.0; 3], now);
        }
    }
    let v = w.view;
    if sx > v.x && sx < v.x + v.w as f64 && sy > v.y && sy < v.y + v.h as f64 && dist > 0.0 {
        let mut y = dy * (30.0 / dist);
        let mut x = dx * (30.0 / dist);
        let n = if hit == 2 {
            x = -x;
            y = -y;
            w.rng.randint(13, 20)
        } else {
            w.rng.randint(8, 10)
        };
        if detail == 3 {
            let p = SnowParticles::new(sx, sy, (x, y), n, crate::gfx::time(), &mut w.rng);
            w.particles.push(p);
        }
    }
    true
}

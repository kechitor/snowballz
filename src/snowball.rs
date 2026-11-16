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

impl Snowball {
    /// Throw at a penguin, with some inaccuracy from far away.
    pub fn throw_at_target(w: &mut World, ui: usize, target: usize) -> Snowball {
        let (px, py) = (w.units[ui].x, w.units[ui].y);
        let (tx, ty) = (w.units[target].x, w.units[target].y);
        let (mut ex, mut ey) = if px <= tx + 3 && px >= tx - 3 && py <= ty + 3 && py >= ty - 3 {
            (tx, ty)
        } else {
            (w.rng.randint(tx - 1, tx + 1), w.rng.randint(ty - 1, ty + 1))
        };
        if w.tile_index(ex, ey).is_none() {
            ex = tx;
            ey = ty;
        }
        let end_tile = w.tile_index(ex, ey).unwrap();
        let end_x = (ex * 32 + w.rng.randint(0, 20)) as f64;
        let end_y = (ey * 32 + w.rng.randint(0, 20)) as f64 - w.tiles[end_tile].elevation * ELEV_MULT;
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

/// Move snowball `i`; true when it has landed (it is then removed by the
/// caller).
pub fn move_snowball(w: &mut World, i: usize) -> bool {
    let now = w.now();
    let s = &mut w.snowballs[i];
    let td = now.wrapping_sub(s.start_time) as f64;
    if s.dist > 0.0 {
        let to_move = td * SPEED;
        let mx = to_move * s.dist_x / s.dist;
        let my = to_move * s.dist_y / s.dist;
        let offset = -((to_move - s.dist / 2.0) / 10.0).powi(2) + s.height;
        s.x = s.start_x + mx;
        s.y = s.start_y + my + 8.0 - offset / 2.0;
        s.shadow_x = s.start_x + mx;
        s.shadow_y = s.start_y + my;
    }
    if td < s.total_time {
        return false;
    }
    // landed
    let (end_tile, start_tile, penguin) = (s.end_tile, s.start_tile, s.penguin);
    let (sx, sy, dx, dy, dist) = (s.x, s.y, s.dist_x, s.dist_y, s.dist);
    let mut hit = 1;
    if w.tiles[end_tile].resource.is_some() {
        hit = 0;
    } else {
        let (tx, ty) = (w.tiles[end_tile].x, w.tiles[end_tile].y);
        for u in w.units_around(tx, ty) {
            if w.units[u].warmth > 0 && w.units[u].state != State::InBuilding {
                let v = unit::detect_hit(w, u, end_tile, start_tile, penguin);
                if v == 2 {
                    hit = 2;
                }
            }
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
